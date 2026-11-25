//! Textteil und mailto-Knoepfe. Kein HTML; Gestaltung folgt in TF4.

use super::code::betreff_marke;
use super::modell::{gueltige_adresse, Umfrage, Vorschlag};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct AntwortLink {
    pub label: String,
    pub url: String,
    pub body: String,
}

/// RFC 6068: UTF-8 pro Byte kodieren, Leerraum als %20, Body-Zeilen als CRLF.
/// Nur ein Empfaenger und die festen Header subject/body sind erlaubt.
pub fn mailto_link(adresse: &str, betreff: &str, body: &str) -> Result<String, &'static str> {
    if !gueltige_adresse(adresse) || betreff.chars().any(char::is_control) {
        return Err("mailto_eingabe");
    }
    let body = body
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n");
    // @ bleibt als Trennzeichen stehen. Alle anderen reservierten Zeichen auch
    // im lokalen Teil kodieren, damit keine URI-Parameter eingeschleust werden.
    let (lokal, domain) = adresse.trim().split_once('@').ok_or("mailto_adresse")?;
    Ok(format!(
        "mailto:{}@{}?subject={}&body={}",
        kodiert(lokal),
        domain,
        kodiert(betreff),
        kodiert(&body)
    ))
}

fn kodiert(text: &str) -> String {
    use std::fmt::Write;
    let mut aus = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            aus.push(char::from(b));
        } else {
            let _ = write!(aus, "%{b:02X}");
        }
    }
    aus
}

fn vorschlag_text(v: &Vorschlag) -> String {
    match v.ortszeit {
        Some(z) => format!(
            "{:02}.{:02}.{:04} {:02}:{:02} (Ortszeit der Umfrage; {})",
            z.tag,
            z.monat,
            z.jahr,
            z.stunde,
            z.minute,
            utc_text(v.start)
        ),
        // Keine stillschweigende Umrechnung in die Zeitzone des Rechners.
        None => format!("{} bis {}", utc_text(v.start), utc_text(v.ende)),
    }
}

/// Gregorianisches Datum in UTC, ohne Rechner-Zeitzone oder neue Abhaengigkeit.
/// Ganze 400-Jahres-Zyklen begrenzen die Schleife auf maximal 399 Jahre.
pub(crate) fn utc_text(ms: i64) -> String {
    let tage = ms.div_euclid(86_400_000);
    if !(-719_162..=2_932_896).contains(&tage) {
        return "Zeitpunkt außerhalb des Datumsbereichs".into();
    }
    let ab_eins = tage + 719_162;
    let mut jahr = 1 + ab_eins.div_euclid(146_097) * 400;
    let mut rest = ab_eins.rem_euclid(146_097);
    let schaltjahr = |j: i64| j % 4 == 0 && (j % 100 != 0 || j % 400 == 0);
    loop {
        let laenge = if schaltjahr(jahr) { 366 } else { 365 };
        if rest < laenge {
            break;
        }
        rest -= laenge;
        jahr += 1;
    }
    let monate = [
        31,
        if schaltjahr(jahr) { 29 } else { 28 },
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
    let mut monat = 1;
    for laenge in monate {
        if rest < laenge {
            break;
        }
        rest -= laenge;
        monat += 1;
    }
    let minuten = ms.rem_euclid(86_400_000) / 60_000;
    format!(
        "{:02}.{monat:02}.{jahr:04} {:02}:{:02} UTC",
        rest + 1,
        minuten / 60,
        minuten % 60
    )
}

/// Empfaengerbezogener Text ohne Teilnehmerliste, Plan-ID oder Agenda.
/// Die Anzeigezeit muss TF2 in der fuer die Umfrage gewaehlten Zeitzone liefern.
pub fn anfrage_text(umfrage: &Umfrage, extern_: bool) -> Result<String, &'static str> {
    umfrage.validiere()?;
    let titel = if extern_ && !umfrage.titel_an_externe {
        "Terminabstimmung".into()
    } else {
        umfrage
            .titel
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut text = format!(
        "{titel}\n\nBitte wählen Sie passende Vorschläge.\nDauer: {} Minuten.\n",
        umfrage.dauer_min
    );
    for v in &umfrage.vorschlaege {
        text.push_str(&format!("{}: {}\n", v.nr, vorschlag_text(v)));
    }
    text.push_str(&format!("\nAntwortfrist: {}.\n", utc_text(umfrage.frist)));
    text.push_str("Antwortmöglichkeiten: Alle Termine passen; Vorschlag N passt; Auswahl beantworten; Keiner passt.\n\
        Bei der Auswahl [ ] durch [x] ersetzen. Freitext wird zur Bestätigung vorgelegt.\n");
    Ok(text)
}

/// `adresse` ist die Rueckantwortadresse der einladenden Person; der persoenliche
/// Code wurde fuer den jeweiligen Empfaenger berechnet, nicht fuer diese Adresse.
pub fn antwort_links(
    adresse: &str,
    umfrage: &Umfrage,
    code: &str,
) -> Result<Vec<AntwortLink>, &'static str> {
    umfrage.validiere()?;
    let betreff = betreff_marke(&umfrage.id, code)?;
    let mut knoepfe = vec![(
        "Alle Termine passen".to_string(),
        "Alle Termine passen".to_string(),
    )];
    for v in &umfrage.vorschlaege {
        let body = format!("Vorschlag {} passt", v.nr);
        knoepfe.push((format!("Vorschlag {}: Passt", v.nr), body));
    }
    let auswahl = umfrage
        .vorschlaege
        .iter()
        .map(|v| match v.ortszeit {
            Some(z) => format!(
                "[ ] {} {:02}.{:02}.{:04} {:02}:{:02}",
                v.nr, z.tag, z.monat, z.jahr, z.stunde, z.minute
            ),
            None => format!("[ ] {}", v.nr),
        })
        .collect::<Vec<_>>()
        .join("\r\n");
    knoepfe.push(("Auswahl beantworten".into(), auswahl));
    knoepfe.push(("Keiner passt".into(), "Keiner passt".into()));
    knoepfe
        .into_iter()
        .map(|(label, body)| {
            Ok(AntwortLink {
                label,
                url: mailto_link(adresse, &betreff, &body)?,
                body,
            })
        })
        .collect()
}
