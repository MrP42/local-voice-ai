//! Sperren des Besprechungsassistenten (G9a, lva#578, wai-sdk ADR-0031 E2.5–E2.10).
//!
//! Reine Entscheidungslogik ohne Uhr, Datei oder Netz. Die App ruft [`pruefen`] VOR JEDEM
//! Versand von Besprechungsinhalten an ein Modell (Live-Chat, laufender Stand, Auswertung,
//! MCP-Ausgabe) und bei jedem Wechsel der Teilnehmerliste erneut.
//!
//! Regeln, alle sperrend:
//! - Ohne eingeschalteten Assistenten, ohne festgelegte Aufbewahrung oder ohne
//!   dokumentierte Freigabe keine Verarbeitung, auch nicht lokal.
//! - Cloud nur, wenn Kundenprofil und Assistentenprofil sie nicht sperren, der Anbieter
//!   freigegeben ist UND die Cloud für diese Besprechung ausdrücklich gewählt wurde.
//! - Einstufung der Teilnehmenden: Extern vor Unbekannt vor Intern. Unbekannt sperrt die
//!   Cloud immer; Extern nur mit der Einstellung „Cloud-Modelle in Besprechungen mit
//!   Externen“. Intern braucht eine Bestätigung, die zur AKTUELLEN Teilnehmerliste passt
//!   (Fingerabdruck); jede Änderung der Liste macht sie ungültig.
//! - Eine Bestätigung kann die berechnete Einstufung nur verschärfen, nie lockern
//!   (Unbekannt lässt sich nicht als „intern“ bestätigen).
//! - Es gibt keinen automatischen Wechsel in die Cloud: fehlt eine zulässige Quelle,
//!   entscheidet die App zwischen lokal und gesperrt, nie diese Logik.

use serde::{Deserialize, Serialize};

use crate::consent::identitaet;

/// Einstufung einer Besprechung nach ihren Teilnehmenden. Die Reihenfolge der Varianten
/// ist die Strenge: ein größerer Wert ist strenger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Einstufung {
    /// Alle Teilnehmenden haben eine Adresse aus den eigenen Domänen (laut Einladung).
    Intern,
    /// Mindestens eine Person ohne verwertbare Adresse, keine Liste (Ad-hoc) oder keine
    /// eigenen Domänen hinterlegt.
    Unbekannt,
    /// Mindestens eine Adresse außerhalb der eigenen Domänen.
    Extern,
}

/// Ergebnis von [`einstufen`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct EinstufungsErgebnis {
    pub einstufung: Einstufung,
    /// Adressen außerhalb der eigenen Domänen (normalisiert, ohne Doppelte).
    pub externe: Vec<String>,
    /// Zahl der Teilnehmenden ohne verwertbare Adresse.
    pub ohne_adresse: u32,
    /// Fingerabdruck der Teilnehmerliste, an den eine Bestätigung gebunden wird.
    pub fingerabdruck: String,
}

/// Normalisiert eine Domänenangabe (`@Example.ORG ` → `example.org`); `None`, wenn sie
/// keine Domäne ist.
pub fn domaene_normalisieren(s: &str) -> Option<String> {
    let t = s.trim().trim_start_matches('@').trim().to_lowercase();
    let ok = t.contains('.')
        && !t.starts_with('.')
        && !t.ends_with('.')
        && !t.contains("..")
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    ok.then_some(t)
}

/// Vorschlag für „Eigene Domänen“ aus den eigenen Adressen (`meeting_self_emails`).
pub fn eigene_domaenen_vorschlag(eigene_adressen: &[String]) -> Vec<String> {
    let mut out: Vec<String> = eigene_adressen
        .iter()
        .filter_map(|a| identitaet(a))
        .filter_map(|p| p.split_once('@').map(|(_, d)| d.to_string()))
        .filter_map(|d| domaene_normalisieren(&d))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Gehört `domaene` zu einer der eigenen Domänen (gleich oder Subdomäne)?
fn ist_eigen(domaene: &str, eigene: &[String]) -> bool {
    eigene.iter().any(|e| {
        domaene == e
            || domaene
                .strip_suffix(e.as_str())
                .is_some_and(|rest| rest.ends_with('.'))
    })
}

/// FNV-1a (64 Bit) als Hex: stabil über Versionen und Plattformen, ohne Abhängigkeit.
fn fnv1a_hex(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Fingerabdruck einer Teilnehmerliste: Reihenfolge und Schreibweise egal, jede
/// hinzugekommene oder entfernte Person ändert ihn. Personen ohne Identität zählen
/// einzeln mit (zwei Unbekannte sind nicht eine).
pub fn fingerabdruck(teilnehmende: &[String]) -> String {
    let mut bekannt: Vec<String> = Vec::new();
    let mut unbekannt = 0usize;
    for t in teilnehmende {
        match identitaet(t) {
            Some(p) => bekannt.push(p),
            None => unbekannt += 1,
        }
    }
    bekannt.sort();
    bekannt.dedup();
    fnv1a_hex(&format!("{}|?{unbekannt}", bekannt.join(";")))
}

/// Stuft eine Besprechung nach ihrer Teilnehmerliste ein (laut Einladung, nicht laut
/// tatsächlicher Anwesenheit).
pub fn einstufen(teilnehmende: &[String], eigene_domaenen: &[String]) -> EinstufungsErgebnis {
    let eigene: Vec<String> = eigene_domaenen
        .iter()
        .filter_map(|d| domaene_normalisieren(d))
        .collect();
    let mut externe: Vec<String> = Vec::new();
    let mut ohne_adresse: u32 = 0;
    let mut intern_gesehen = false;
    for t in teilnehmende {
        let domaene =
            identitaet(t).and_then(|p| p.split_once('@').map(|(_, d)| (p.clone(), d.to_string())));
        match domaene {
            None => ohne_adresse += 1,
            Some((p, d)) => {
                if eigene.is_empty() {
                    // Ohne eigene Domänen ist keine Adresse als intern belegbar.
                    ohne_adresse += 1;
                } else if ist_eigen(&d, &eigene) {
                    intern_gesehen = true;
                } else if !externe.contains(&p) {
                    externe.push(p);
                }
            }
        }
    }
    externe.sort();
    let einstufung = if !externe.is_empty() {
        Einstufung::Extern
    } else if ohne_adresse > 0 || !intern_gesehen {
        Einstufung::Unbekannt
    } else {
        Einstufung::Intern
    };
    EinstufungsErgebnis {
        einstufung,
        externe,
        ohne_adresse,
        fingerabdruck: fingerabdruck(teilnehmende),
    }
}

/// Bestätigung der Einstufung durch den Nutzer beim Start oder nach einem Wechsel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Bestaetigung {
    /// Vom Nutzer bestätigte Einstufung; zählt nur, wenn sie nicht lockerer ist als die
    /// berechnete.
    pub einstufung: Einstufung,
    /// Fingerabdruck der Teilnehmerliste zum Zeitpunkt der Bestätigung.
    pub fingerabdruck: String,
}

/// Stand der dokumentierten Freigabe (Information aller Teilnehmenden, ggf. Einwilligung).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Freigabe {
    Erteilt,
    Offen,
    Abgelehnt,
}

/// Wohin der Inhalt gehen soll.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Quelle {
    Lokal,
    Cloud,
}

/// Alles, was die App für eine Prüfung zusammenträgt. Fehlende Angaben setzt die App
/// auf den strengen Wert (`false`, `Offen`, `None`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Anfrage {
    /// Schalter „Assistent in der Besprechung“.
    pub assistent_an: bool,
    /// Aufbewahrungsregel für alle Datenarten festgelegt (lva#536).
    pub aufbewahrung_festgelegt: bool,
    pub freigabe: Freigabe,
    pub quelle: Quelle,
    /// Kundenprofil sperrt Cloud-Quellen.
    pub kundenprofil_nur_lokal: bool,
    /// Gewähltes Assistentenprofil ist „nur lokal“ (z. B. Personalbezug).
    pub profil_nur_lokal: bool,
    /// Anbieter freigegeben und regelkonform (Freigabe je Anbieter, ADR-0028).
    pub anbieter_freigegeben: bool,
    /// Cloud für DIESE Besprechung ausdrücklich gewählt.
    pub cloud_fuer_besprechung: bool,
    /// Einstellung „Cloud-Modelle in Besprechungen mit Externen“ (Standard aus).
    pub cloud_bei_externen: bool,
    /// Aktuelle Einstufung aus [`einstufen`].
    pub einstufung: EinstufungsErgebnis,
    pub bestaetigung: Option<Bestaetigung>,
}

/// Warum eine Verarbeitung gesperrt ist. Die App zeigt alle Gründe, nicht nur den ersten.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Sperrgrund {
    AssistentAus,
    AufbewahrungOffen,
    FreigabeOffen,
    FreigabeAbgelehnt,
    KundenprofilNurLokal,
    ProfilNurLokal,
    AnbieterNichtFreigegeben,
    CloudNichtGewaehlt,
    EinstufungUnbestaetigt,
    EinstufungVeraltet,
    TeilnehmendeUnbekannt,
    TeilnehmendeExtern,
}

/// Ergebnis von [`pruefen`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "ergebnis", rename_all = "snake_case")]
pub enum Entscheidung {
    Erlaubt {
        /// Wirksame Einstufung (die strengere aus Berechnung und Bestätigung).
        einstufung: Einstufung,
    },
    Gesperrt {
        gruende: Vec<Sperrgrund>,
    },
}

impl Entscheidung {
    pub fn erlaubt(&self) -> bool {
        matches!(self, Entscheidung::Erlaubt { .. })
    }
}

/// Wirksame Einstufung: die strengere aus Berechnung und gültiger Bestätigung. Eine
/// Bestätigung zu einer anderen Teilnehmerliste zählt nicht.
pub fn wirksame_einstufung(
    ergebnis: &EinstufungsErgebnis,
    bestaetigung: Option<&Bestaetigung>,
) -> Einstufung {
    match bestaetigung {
        Some(b) if b.fingerabdruck == ergebnis.fingerabdruck => {
            ergebnis.einstufung.max(b.einstufung)
        }
        _ => ergebnis.einstufung,
    }
}

/// Entscheidet, ob Besprechungsinhalte an die angefragte Quelle gehen dürfen.
pub fn pruefen(a: &Anfrage) -> Entscheidung {
    let mut gruende = Vec::new();
    if !a.assistent_an {
        gruende.push(Sperrgrund::AssistentAus);
    }
    if !a.aufbewahrung_festgelegt {
        gruende.push(Sperrgrund::AufbewahrungOffen);
    }
    match a.freigabe {
        Freigabe::Erteilt => {}
        Freigabe::Offen => gruende.push(Sperrgrund::FreigabeOffen),
        Freigabe::Abgelehnt => gruende.push(Sperrgrund::FreigabeAbgelehnt),
    }
    let wirksam = wirksame_einstufung(&a.einstufung, a.bestaetigung.as_ref());
    if a.quelle == Quelle::Cloud {
        if a.kundenprofil_nur_lokal {
            gruende.push(Sperrgrund::KundenprofilNurLokal);
        }
        if a.profil_nur_lokal {
            gruende.push(Sperrgrund::ProfilNurLokal);
        }
        if !a.anbieter_freigegeben {
            gruende.push(Sperrgrund::AnbieterNichtFreigegeben);
        }
        if !a.cloud_fuer_besprechung {
            gruende.push(Sperrgrund::CloudNichtGewaehlt);
        }
        match &a.bestaetigung {
            None => gruende.push(Sperrgrund::EinstufungUnbestaetigt),
            Some(b) if b.fingerabdruck != a.einstufung.fingerabdruck => {
                gruende.push(Sperrgrund::EinstufungVeraltet)
            }
            Some(_) => {}
        }
        match wirksam {
            Einstufung::Intern => {}
            Einstufung::Unbekannt => gruende.push(Sperrgrund::TeilnehmendeUnbekannt),
            Einstufung::Extern if !a.cloud_bei_externen => {
                gruende.push(Sperrgrund::TeilnehmendeExtern)
            }
            Einstufung::Extern => {}
        }
    }
    if gruende.is_empty() {
        Entscheidung::Erlaubt {
            einstufung: wirksam,
        }
    } else {
        Entscheidung::Gesperrt { gruende }
    }
}
