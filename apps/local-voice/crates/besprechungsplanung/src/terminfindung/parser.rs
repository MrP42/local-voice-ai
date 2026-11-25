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
    let original = text.replace("\r\n", "\n").replace('\r', "\n");
    let norm = original.to_lowercase();
    let mut zeilen = Vec::new();
    let mut signatur = Vec::new();
    let mut in_signatur = false;
    for (originalzeile, zeile) in original.lines().zip(norm.lines()) {
        let originalzeile = originalzeile.trim();
        let zeile = zeile.trim();
        if verlauf_beginn(zeile) {
            break;
        }
        // Nur Zitatzeilen auslassen; Inline-Antworten danach gehoeren zur
        // neuen Nachricht. Eine Signatur darf keinen Widerspruch verdecken.
        if zeile.starts_with('>') {
            continue;
        }
        in_signatur |= signatur_beginn(zeile);
        if in_signatur {
            signatur.push(originalzeile);
        } else if !zeile.is_empty() {
            zeilen.push(zeile);
        }
    }
    if zeilen.iter().take(3).any(|z| abwesenheit(z)) {
        return leer(ErgebnisArt::Ignorieren, "Automatische Abwesenheitsantwort.");
    }
    if signatur_antwortgehalt(&signatur) {
        return leer(ErgebnisArt::Pruefen, "Antwortgehalt in Signatur: Bitte pruefen.");
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

fn signatur_beginn(z: &str) -> bool {
    let gruss = z.trim_end_matches([',', '.', '!']).trim_end();
    z == "--"
        || (z.len() >= 5 && z.bytes().all(|b| b == b'_' || b == b'-'))
        || z.starts_with("gesendet von ")
        || z.starts_with("sent from ")
        || [
            "mit freundlichen grüßen",
            "mit freundlichen grüssen",
            "mit freundlichen gruessen",
            "viele grüße",
            "viele grüsse",
            "viele gruesse",
            "beste grüße",
            "beste grüsse",
            "beste gruesse",
            "best regards",
            "kind regards",
        ]
        .contains(&gruss)
}

fn signatur_antwortgehalt(zeilen: &[&str]) -> bool {
    // Nur eindeutig erkannte Signaturteile ausnehmen. Unbekannter Text ist
    // keine Signatur, auch wenn noch kein bekanntes Absagemuster darin steht.
    for (i, zeile) in zeilen.iter().enumerate() {
        let norm = zeile.to_lowercase();
        if [
            "keine haftung für ",
            "keine haftung fuer ",
            "diese e-mail ist vertraulich",
            "diese nachricht ist vertraulich",
            "this email is confidential",
            "this e-mail is confidential",
            "no liability for ",
            "we do not accept liability for ",
        ]
        .iter()
        .any(|p| norm.starts_with(p))
        {
            return !hinweisblock_harmlos(&zeilen[i..]);
        }
        if !signaturzeile_harmlos(zeile) {
            return true;
        }
    }
    false
}

fn signaturzahl(z: &str) -> bool {
    z.chars().any(|c| c.is_ascii_digit())
        && z.chars()
            .all(|c| c.is_ascii_digit() || " +()-./".contains(c))
}

fn signaturname(z: &str) -> bool {
    let woerter: Vec<_> = z.split_whitespace().collect();
    (1..=5).contains(&woerter.len())
        && woerter
            .iter()
            .any(|w| w.chars().next().is_some_and(char::is_uppercase))
        && woerter.iter().all(|w| {
            let norm = w.to_lowercase();
            if ["von", "van", "de", "der", "zu", "und", "&", "dr.", "prof."]
                .contains(&norm.as_str())
            {
                return true;
            }
            // Auch grossgeschriebene Satzanfaenge sind keine Namenswoerter.
            ![
                "ich", "wir", "i", "we", "ps", "ja", "nein", "yes", "no", "leider",
                "kann", "können", "koennen", "muss", "müssen", "muessen", "nicht",
                "absagen", "teilnehmen", "passt", "passen", "geht", "komme", "sage",
                "bin", "sind", "werde", "werden", "verschieben", "kommen", "nehmen",
                "can", "cannot", "must", "not", "cancel", "attend", "accept", "work",
            ]
            .contains(&norm.as_str())
                && w.chars().next().is_some_and(char::is_uppercase)
                && w.chars().all(|c| c.is_alphabetic() || "-'’".contains(c))
        })
}

fn signaturzeile_harmlos(z: &str) -> bool {
    let norm = z.to_lowercase();
    if z.is_empty()
        || (signatur_beginn(&norm)
            && !norm.starts_with("gesendet von ")
            && !norm.starts_with("sent from "))
    {
        return true;
    }
    if [
        "gesendet von meinem smartphone",
        "gesendet von meinem iphone",
        "gesendet von meinem ipad",
        "sent from my phone",
        "sent from my iphone",
        "sent from my ipad",
        "sent from outlook for ios",
        "sent from outlook for android",
    ]
    .contains(&norm.as_str())
    {
        return true;
    }
    if signaturzahl(z) {
        return true;
    }
    for praefix in [
        "telefon:", "tel:", "tel.", "phone:", "fax:", "mobil:", "mobile:", "stand:",
    ] {
        if let Some(rest) = norm.strip_prefix(praefix) {
            return signaturzahl(rest.trim());
        }
    }
    let kontakt = norm
        .strip_prefix("e-mail:")
        .or_else(|| norm.strip_prefix("email:"))
        .or_else(|| norm.strip_prefix("mail:"))
        .unwrap_or(&norm)
        .trim();
    if !kontakt.chars().any(char::is_whitespace) {
        if let Some((lokal, domain)) = kontakt.split_once('@') {
            if !lokal.is_empty()
                && domain.contains('.')
                && kontakt.chars().all(|c| c.is_alphanumeric() || "@._+-".contains(c))
            {
                return true;
            }
        }
        if ["https://", "http://", "www."]
            .iter()
            .any(|p| kontakt.starts_with(p))
            && kontakt.contains('.')
            && kontakt.chars().all(|c| c.is_alphanumeric() || ":/._~%-".contains(c))
        {
            return true;
        }
    }
    for praefix in ["handelsregister:", "ust-idnr.:", "ust-idnr:", "vat:"] {
        if let Some(rest) = norm.strip_prefix(praefix) {
            let rest = rest.trim();
            let nummer = rest
                .strip_prefix("hrb")
                .or_else(|| rest.strip_prefix("hra"))
                .or_else(|| rest.strip_prefix("de"))
                .unwrap_or(rest)
                .trim();
            return !nummer.is_empty() && nummer.bytes().all(|b| b.is_ascii_digit());
        }
    }
    let woerter: Vec<_> = z.split_whitespace().collect();
    let namen: Vec<_> = woerter
        .iter()
        .copied()
        .filter(|w| !w.bytes().all(|b| b.is_ascii_digit()))
        .collect();
    if namen.len() < woerter.len() {
        // PLZ oder Hausnummer nur zusammen mit einem Orts-/Strassennamen.
        return signaturname(&namen.join(" "))
            && (woerter
                .first()
                .is_some_and(|w| w.len() == 5 && w.bytes().all(|b| b.is_ascii_digit()))
                || namen.iter().any(|w| {
                    let w = w.to_lowercase();
                    ["straße", "strasse", "str.", "weg", "platz", "road", "street"]
                        .iter()
                        .any(|suffix| w.ends_with(suffix))
                }));
    }
    signaturname(z)
}

fn hinweisblock_harmlos(zeilen: &[&str]) -> bool {
    let text = zeilen.join(" ").to_lowercase();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    // Der Hinweis gilt bis zum Textende. Nachschriften und persoenliche
    // Aussagen bleiben gesperrt; nur feste Haftungsformeln nehmen wir aus.
    let text = text
        .replace("we do not accept liability for transmission errors.", "")
        .replace("i cannot accept liability for damage.", "");
    !text
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| ["ps", "ich", "wir", "i", "we"].contains(&w))
}

fn verlauf_beginn(z: &str) -> bool {
    z.starts_with("von:")
        || z.starts_with("from:")
        || z.starts_with("-----original message")
        || z.starts_with("-----ursprüngliche nachricht")
        || z.starts_with("-----urspruengliche nachricht")
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
