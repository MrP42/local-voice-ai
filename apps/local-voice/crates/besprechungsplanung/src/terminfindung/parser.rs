//! Begrenzter Klartext-Parser ohne Regex und ohne Sprachmodell. Freitext wird
//! nur als Vorschlag geliefert; der Aufrufer muss ihn bestaetigen lassen.

use super::modell::{Ortszeit, Quelle, Umfrage, Wert};
use serde::{Deserialize, Serialize};

pub const MAX_ANTWORT_BYTES: usize = 32_768;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ErgebnisArt {
    Antwort,
    Ignorieren,
    Pruefen,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AntwortWert {
    pub vorschlag_nr: u8,
    pub wert: Wert,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Ergebnis {
    pub art: ErgebnisArt,
    pub werte: Vec<AntwortWert>,
    pub quelle: Option<Quelle>,
    pub sicher: bool,
    /// Fester Hinweis ohne Mailinhalt oder personenbezogene Daten.
    pub grund: String,
}

fn leer(art: ErgebnisArt, grund: &str) -> Ergebnis {
    Ergebnis {
        art,
        werte: vec![],
        quelle: None,
        sicher: false,
        grund: grund.into(),
    }
}

fn antwort(u: &Umfrage, auswahl: &[(u8, Wert)], sonst: Wert, quelle: Quelle) -> Ergebnis {
    Ergebnis {
        art: ErgebnisArt::Antwort,
        werte: u
            .vorschlaege
            .iter()
            .map(|v| AntwortWert {
                vorschlag_nr: v.nr,
                wert: auswahl
                    .iter()
                    .find(|(nr, _)| *nr == v.nr)
                    .map_or(sonst, |(_, w)| *w),
            })
            .collect(),
        quelle: Some(quelle),
        sicher: quelle != Quelle::Freitext,
        grund: if quelle == Quelle::Freitext {
            "Freitext: Bitte bestaetigen."
        } else {
            "Strukturierte Antwort erkannt; Absender und Code separat pruefen."
        }
        .into(),
    }
}

/// HTML muss der Adapter vorher in Klartext wandeln. Kein stilles Abschneiden
/// ueberlanger Nachrichten: am Ende koennte ein Widerspruch stehen.
pub fn lese_antwort(text: &str, umfrage: &Umfrage) -> Ergebnis {
    if text.len() > MAX_ANTWORT_BYTES {
        return leer(ErgebnisArt::Pruefen, "Antwort zu lang.");
    }
    if umfrage.validiere().is_err() {
        return leer(ErgebnisArt::Pruefen, "Umfrage ungueltig.");
    }
    let norm = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .to_lowercase();
    let mut zeilen = Vec::new();
    for zeile in norm.lines().map(str::trim).filter(|z| !z.is_empty()) {
        if zitat_beginn(zeile) {
            break;
        }
        zeilen.push(zeile);
    }
    if zeilen.iter().take(3).any(|z| abwesenheit(z)) {
        return leer(ErgebnisArt::Ignorieren, "Automatische Abwesenheitsantwort.");
    }
    if zeilen.iter().any(|z| {
        z.contains("weitergeleitet")
            || z.contains("forwarded")
            || z.starts_with("fwd:")
            || z.starts_with("fw:")
            || z.starts_with("wg:")
    }) {
        return leer(ErgebnisArt::Pruefen, "Weiterleitung: Bitte pruefen.");
    }
    if zeilen.is_empty() {
        return leer(ErgebnisArt::Pruefen, "Keine neue Antwort.");
    }
    if zeilen.iter().any(|z| z.starts_with('[')) {
        let mut auswahl = Vec::new();
        for z in &zeilen {
            let (wert, rest) = if let Some(r) = z.strip_prefix("[x]") {
                (Wert::Passt, r)
            } else if let Some(r) = z.strip_prefix("[ ]") {
                (Wert::PasstNicht, r)
            } else {
                return leer(ErgebnisArt::Pruefen, "Auswahl unklar.");
            };
            let rest = rest.trim();
            let (nummer, beschriftung) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            let nr = vorschlag_nummer(nummer);
            let Some(nr) = nr.filter(|nr| umfrage.vorschlaege.iter().any(|v| v.nr == *nr)) else {
                return leer(ErgebnisArt::Pruefen, "Unbekannter Vorschlag.");
            };
            // Nur Datum/Uhrzeit als Beschriftung akzeptieren. Ein nachgestelltes
            // "passt nicht" darf eine angekreuzte Zeile nicht sicher machen.
            if !beschriftung.is_empty()
                && !umfrage.vorschlaege.iter().any(|v| {
                    v.nr == nr
                        && v.ortszeit.is_some_and(|z| {
                            ZeitMuster::parse(beschriftung)
                                .is_some_and(|m| m.passt(z, umfrage.dauer_min))
                        })
                })
            {
                return leer(ErgebnisArt::Pruefen, "Auswahlbeschriftung unklar.");
            }
            if auswahl.iter().any(|(n, _)| *n == nr) {
                return leer(ErgebnisArt::Pruefen, "Doppelte Auswahl.");
            }
            auswahl.push((nr, wert));
        }
        if !auswahl.iter().any(|(_, w)| *w == Wert::Passt) {
            return leer(ErgebnisArt::Pruefen, "Leere Auswahl: Bitte bestaetigen.");
        }
        return antwort(umfrage, &auswahl, Wert::Offen, Quelle::Auswahl);
    }
    let text = zeilen.join(" ");
    let text = text.trim_end_matches(['.', '!']).trim();
    match text {
        "alle termine passen" | "all dates work" | "all options work" => {
            return antwort(umfrage, &[], Wert::Passt, Quelle::Knopf)
        }
        "keiner passt" | "none work" | "no dates work" => {
            return antwort(umfrage, &[], Wert::PasstNicht, Quelle::Knopf)
        }
        "ja" | "yes" => return antwort(umfrage, &[], Wert::Passt, Quelle::Freitext),
        "nein" | "no" | "geht nicht" | "does not work" => {
            return antwort(umfrage, &[], Wert::PasstNicht, Quelle::Freitext)
        }
        _ => {}
    }
    for (praefix, suffix) in [("vorschlag ", " passt"), ("option ", " works")] {
        if let Some(nr) = text
            .strip_prefix(praefix)
            .and_then(|s| s.strip_suffix(suffix))
            .and_then(vorschlag_nummer)
            .filter(|nr| umfrage.vorschlaege.iter().any(|v| v.nr == *nr))
        {
            return antwort(umfrage, &[(nr, Wert::Passt)], Wert::Offen, Quelle::Knopf);
        }
    }
    let (nur, liste) = if let Some(s) = text
        .strip_prefix("nur ")
        .or_else(|| text.strip_prefix("only "))
    {
        (true, s)
    } else {
        (false, text)
    };
    let (listenwert, nummerntext) = if let Some(s) = liste
        .strip_suffix(" notfalls")
        .or_else(|| liste.strip_suffix(" if necessary"))
    {
        (Wert::Notfalls, s)
    } else {
        (Wert::Passt, liste)
    };
    if let Some(nummern) = nummernliste(nummerntext, umfrage) {
        let werte: Vec<_> = nummern.into_iter().map(|nr| (nr, listenwert)).collect();
        return antwort(
            umfrage,
            &werte,
            if nur { Wert::PasstNicht } else { Wert::Offen },
            Quelle::Freitext,
        );
    }
    if let Some(zeit) = ZeitMuster::parse(liste) {
        let treffer: Vec<_> = umfrage
            .vorschlaege
            .iter()
            .filter(|v| v.ortszeit.is_some_and(|z| zeit.passt(z, umfrage.dauer_min)))
            .map(|v| (v.nr, Wert::Passt))
            .collect();
        // Uhrzeit oder Wochentag allein muss einen einzigen Vorschlag meinen.
        if treffer.len() == 1 {
            return antwort(
                umfrage,
                &treffer,
                if nur { Wert::PasstNicht } else { Wert::Offen },
                Quelle::Freitext,
            );
        }
    }
    leer(ErgebnisArt::Pruefen, "Antwort unklar: Bitte pruefen.")
}

fn zitat_beginn(z: &str) -> bool {
    z == "--"
        || z.starts_with('>')
        || z.starts_with("von:")
        || z.starts_with("from:")
        || z.starts_with("-----original message")
        || z.starts_with("-----ursprüngliche nachricht")
        || z.starts_with("-----urspruengliche nachricht")
        || (z.len() >= 5 && z.bytes().all(|b| b == b'_' || b == b'-'))
        || (z.starts_with("am ")
            && (z.ends_with("schrieb:") || (z.contains(" schrieb ") && z.ends_with(':'))))
        || (z.starts_with("on ") && z.ends_with("wrote:"))
}

fn abwesenheit(z: &str) -> bool {
    [
        "abwesenheitsnotiz",
        "automatische antwort",
        "automatisch generierte antwort",
        "automatic reply",
        "auto-reply",
        "autoreply",
        "out of office",
        "out-of-office",
    ]
    .iter()
    .any(|p| z.starts_with(p))
}

fn nummernliste(text: &str, u: &Umfrage) -> Option<Vec<u8>> {
    let norm = text.replace([',', ';', '&'], " ");
    let mut nummern = Vec::new();
    for token in norm.split_whitespace() {
        if token == "und" || token == "and" {
            continue;
        }
        let nr = vorschlag_nummer(token)?;
        if !u.vorschlaege.iter().any(|v| v.nr == nr) || nummern.contains(&nr) {
            return None;
        }
        nummern.push(nr);
    }
    (!nummern.is_empty()).then_some(nummern)
}

fn vorschlag_nummer(text: &str) -> Option<u8> {
    if text.len() != 1 {
        return None;
    }
    let b = text.as_bytes()[0];
    (b'1'..=b'8').contains(&b).then(|| b - b'0')
}

#[derive(Default)]
struct ZeitMuster {
    datum: Option<(u8, u8, Option<u16>)>,
    wochentag: Option<u8>,
    uhrzeit: Option<(u8, u8)>,
    dauer_min: Option<u32>,
}

impl ZeitMuster {
    fn parse(text: &str) -> Option<Self> {
        let mut m = Self::default();
        for t in text.split_whitespace() {
            if t == "uhr" && m.uhrzeit.is_some() {
                continue;
            }
            let tage = [
                ["montag", "monday", "mo"],
                ["dienstag", "tuesday", "di"],
                ["mittwoch", "wednesday", "mi"],
                ["donnerstag", "thursday", "do"],
                ["freitag", "friday", "fr"],
                ["samstag", "saturday", "sa"],
                ["sonntag", "sunday", "so"],
            ];
            if let Some(i) = tage
                .iter()
                .position(|namen| namen.contains(&t.trim_end_matches(',')))
            {
                if m.wochentag.replace(i as u8 + 1).is_some() {
                    return None;
                }
            } else if let Some((h, min)) = t.split_once(':') {
                // Outlook stellt Zeitspannen mit Bindestrich oder Halbgeviert dar.
                let (min, ende) = min
                    .split_once(['–', '-'])
                    .map_or((min, None), |(m, e)| (m, Some(e)));
                let h: u8 = h.parse().ok()?;
                let min: u8 = min.parse().ok()?;
                if h > 23 || min > 59 {
                    return None;
                }
                if let Some(ende) = ende {
                    let (eh, em) = ende.split_once(':')?;
                    let eh: u8 = eh.parse().ok()?;
                    let em: u8 = em.parse().ok()?;
                    if eh > 23 || em > 59 {
                        return None;
                    }
                    m.dauer_min = Some(
                        (u32::from(eh) * 60 + u32::from(em) + 1440
                            - u32::from(h) * 60
                            - u32::from(min))
                            % 1440,
                    );
                }
                if m.uhrzeit.replace((h, min)).is_some() {
                    return None;
                }
            } else {
                let teile: Vec<_> = t.trim_end_matches('.').split('.').collect();
                if teile.len() != 2 && teile.len() != 3 {
                    return None;
                }
                let tag: u8 = teile[0].parse().ok()?;
                let monat: u8 = teile[1].parse().ok()?;
                let jahr = if teile.len() == 3 {
                    Some(teile[2].parse::<u16>().ok()?)
                } else {
                    None
                };
                if m.datum.replace((tag, monat, jahr)).is_some() {
                    return None;
                }
            }
        }
        (m.datum.is_some() || m.wochentag.is_some() || m.uhrzeit.is_some()).then_some(m)
    }

    fn passt(&self, z: Ortszeit, dauer_min: u32) -> bool {
        self.datum.is_none_or(|(tag, monat, jahr)| {
            tag == z.tag && monat == z.monat && jahr.is_none_or(|j| j == z.jahr)
        }) && self.wochentag.is_none_or(|t| t == z.wochentag)
            && self
                .uhrzeit
                .is_none_or(|(h, m)| h == z.stunde && m == z.minute)
            && self.dauer_min.is_none_or(|d| d == dauer_min)
    }
}
