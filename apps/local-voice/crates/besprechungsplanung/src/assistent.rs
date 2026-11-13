//! Sperren des Besprechungsassistenten (G9a, lva#578, wai-sdk ADR-0031 E2.5–E2.10).
//!
//! Reine Entscheidungslogik ohne Uhr, Datei oder Netz. Die App ruft [`pruefen`] VOR JEDEM
//! Versand von Besprechungsinhalten an ein Modell (Live-Chat, laufender Stand, Auswertung,
//! MCP-Ausgabe) und bei jedem Wechsel der Teilnehmerliste erneut.
//!
//! Regeln, alle sperrend:
//! - MCP-Ausgabe ([`Pfad::Mcp`]) gilt immer als Cloud: der MCP-Client ist kein Nachweis
//!   lokaler Verarbeitung (E2.8), auch wenn die App ihn als `Quelle::Lokal` anfragt.
//! - Ohne eingeschalteten Assistenten, ohne festgelegte Aufbewahrung oder ohne
//!   dokumentierte Freigabe keine Verarbeitung, auch nicht lokal. Die Freigabe gilt nur für
//!   die Runde, für die sie erteilt wurde: Sie ist an Besprechung, Teilnehmerliste und
//!   Teilnehmer-Revision gebunden ([`freigabe_bindung`]); jede Änderung der Runde (auch eine
//!   eingeladene Person, die später eintritt) pausiert jede Verarbeitung, auch lokal, bis zur
//!   erneuten Freigabe. Diese Prüfung liegt VOR der Unterscheidung lokal/Cloud.
//! - Cloud und MCP nur mit aktuellem, vollständigem Teilnehmerstand
//!   ([`Teilnehmerinfo::AktuellVollstaendig`]); veraltet, unvollständig oder unbekannt sperrt
//!   immer, auch mit „Cloud bei Externen“. Den Stand belegt der Aufrufer (diese Logik hat
//!   keine Uhr).
//! - Cloud nur, wenn Kundenprofil und Assistentenprofil sie nicht sperren, der Anbieter
//!   freigegeben ist UND die Cloud für diese Besprechung ausdrücklich gewählt wurde.
//! - Einstufung der Teilnehmenden: Extern vor Unbekannt vor Intern. Unbekannt sperrt die
//!   Cloud immer; Extern nur mit der Einstellung „Cloud-Modelle in Besprechungen mit
//!   Externen“. Intern braucht eine Bestätigung, die zur AKTUELLEN Teilnehmerliste und den
//!   aktuellen eigenen Domänen passt (Fingerabdruck); jede Änderung macht sie ungültig.
//!   [`pruefen`] stuft selbst ein und vertraut keiner mitgelieferten Einstufung.
//! - Eine Liste, die nicht aus einem Termin stammt (Ad-hoc, getippt), ist nie belegt intern
//!   (E2.6). Ein Eintrag mit mehr als einer Adresse, Trennzeichen oder Nicht-ASCII-Zeichen
//!   in der Adresse zählt als Person ohne Adresse.
//! - Unbekannte Teilnehmende sperren die Cloud auch dann, wenn zugleich Externe dabei sind
//!   und „Cloud bei Externen“ an ist; die Einstellung lockert nur bestätigtes „extern“.
//! - Eigene Domänen gelten genau; Subdomänen nur mit Eintrag `*.example.org`. Ungültige
//!   Domänen zählen nie als eigen. Öffentliche Mail-/Hosting-Domänen und Länder-Suffixe
//!   (`co.at`, `com.au`) weist eine HEURISTIK ab: Beide Listen sind ausdrücklich
//!   unvollständig (keine Public-Suffix-Liste), es gibt keine Eigentumsprüfung. Eine vom
//!   Nutzer eingetragene eigene Domäne bleibt eine Vertrauensannahme; die Oberfläche warnt
//!   beim Übernehmen.
//! - Bestätigungen speichert die App strikt je Besprechung; der Fingerabdruck allein ist
//!   besprechungsübergreifend gleich und kein Nachweis „je Besprechung“.
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
    /// Mindestens eine Person ohne verwertbare Adresse, Liste nicht aus einem Termin
    /// (Ad-hoc), leere Liste oder keine eigenen Domänen hinterlegt.
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
    /// Liste stammt aus einem Termin (sonst Ad-hoc: nie belegt intern).
    pub aus_termin: bool,
    /// Fingerabdruck der Teilnehmerliste, an den eine Bestätigung gebunden wird.
    pub fingerabdruck: String,
}

/// Normalisiert eine Domänenangabe (`@Example.ORG ` → `example.org`); `None`, wenn sie
/// keine gültige ASCII-Domäne ist (nur `a-z0-9-` je Teil, kein leerer Teil, Teil nicht
/// mit `-` am Rand, mindestens zwei Teile).
pub fn domaene_normalisieren(s: &str) -> Option<String> {
    let t = s.trim().trim_start_matches('@').trim();
    if !t.is_ascii() {
        return None;
    }
    let t = t.to_ascii_lowercase();
    let teile: Vec<&str> = t.split('.').collect();
    let ok = teile.len() >= 2
        && teile.iter().all(|teil| {
            !teil.is_empty()
                && teil.len() <= 63
                && !teil.starts_with('-')
                && !teil.ends_with('-')
                && teil.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    ok.then_some(t)
}

/// Öffentliche Mail- und Hosting-Domänen: als eigene Domäne nie zulässig, weil sonst jede
/// fremde Adresse dort als intern gälte. Keine vollständige Liste; sie lehnt das
/// Offensichtliche ab, die Einstellung bleibt eine bewusste Angabe des Nutzers.
const OEFFENTLICHE_DOMAENEN: &[&str] = &[
    "1und1.de",
    "arcor.de",
    "freenet.de",
    "mac.com",
    "mail.ru",
    "mailbox.org",
    "me.com",
    "msn.com",
    "online.de",
    "onmicrosoft.com",
    "pm.me",
    "posteo.de",
    "posteo.net",
    "t-online.de",
    "tuta.com",
    "tuta.io",
    "tutamail.com",
    "tutanota.com",
    "tutanota.de",
    "vodafone.de",
    "web.de",
    "bluewin.ch",
    "email.de",
    "fastmail.com",
    "gmx.li",
    "qq.com",
    "rocketmail.com",
    "co.com",
    "eu.org",
    "uk.com",
    "us.com",
    "amazonaws.com",
    "cloudfront.net",
    "firebaseapp.com",
    "web.app",
    // Hosting: jede Subdomäne gehört jemand anderem.
    "azurewebsites.net",
    "blogspot.com",
    "github.io",
    "herokuapp.com",
    "netlify.app",
    "pages.dev",
    "sharepoint.com",
    "vercel.app",
];

/// Marken öffentlicher Mail-Dienste: `marke.tld` und `marke.co.tld` (z. B. `gmx.com`,
/// `yahoo.fr`, `yahoo.co.uk`) sind nie eigen, gleich unter welcher Landesendung.
const OEFFENTLICHE_MARKEN: &[&str] = &[
    "aol",
    "gmail",
    "gmx",
    "googlemail",
    "hotmail",
    "icloud",
    "live",
    "mail",
    "outlook",
    "proton",
    "protonmail",
    "yahoo",
    "yandex",
    "ymail",
    "zoho",
];

/// Zweite Ebenen unter Länderendungen, unter denen fremde Firmen registrieren
/// (`co.uk`, `co.at`, `com.au`, `or.at`, `gv.at` …).
const ZWEITE_EBENEN: &[&str] = &[
    "ac", "co", "com", "edu", "go", "gob", "gov", "gv", "ltd", "me", "ne", "net", "nom", "or",
    "org", "plc", "sch",
];

/// Ist `d` ein Länder-Suffix wie `co.at` (zwei Teile, Landesendung, bekannte zweite Ebene)?
fn ist_laender_suffix(teile: &[&str]) -> bool {
    teile.len() == 2 && teile[1].len() == 2 && ZWEITE_EBENEN.contains(&teile[0])
}

/// Ist `d` (normalisiert) als eigene Domäne zulässig, mit oder ohne Stern?
fn als_eigene_zulaessig(d: &str) -> bool {
    if OEFFENTLICHE_DOMAENEN.contains(&d) {
        return false;
    }
    let teile: Vec<&str> = d.split('.').collect();
    if ist_laender_suffix(&teile) {
        return false;
    }
    let marke = OEFFENTLICHE_MARKEN.contains(&teile[0]);
    let unter_suffix = teile.len() == 3 && ist_laender_suffix(&teile[1..]);
    !(marke && (teile.len() == 2 || unter_suffix))
}

/// Ein Eintrag der Einstellung „Eigene Domänen“: `example.org` gilt nur genau, `*.example.org`
/// zusätzlich für alle Subdomänen (ADR-0031 E2.6: „steht in der Einstellung“).
#[derive(Clone, Debug, PartialEq, Eq)]
struct EigeneDomaene {
    domaene: String,
    mit_subdomaenen: bool,
}

fn eigene_eintraege(eigene_domaenen: &[String]) -> Vec<EigeneDomaene> {
    let mut out: Vec<EigeneDomaene> = eigene_domaenen
        .iter()
        .filter_map(|e| {
            let t = e.trim();
            let (rest, sub) = match t.strip_prefix("*.") {
                Some(r) => (r, true),
                None => (t, false),
            };
            let d = domaene_normalisieren(rest)?;
            als_eigene_zulaessig(&d).then_some(EigeneDomaene {
                domaene: d,
                mit_subdomaenen: sub,
            })
        })
        .collect();
    out.sort_by(|a, b| (&a.domaene, a.mit_subdomaenen).cmp(&(&b.domaene, b.mit_subdomaenen)));
    out.dedup();
    out
}

/// Vorschlag für „Eigene Domänen“ aus den eigenen Adressen (`meeting_self_emails`). Nur zum
/// Anzeigen; die App übernimmt ihn erst nach Bestätigung. Öffentliche Mail-Domänen fallen weg.
pub fn eigene_domaenen_vorschlag(eigene_adressen: &[String]) -> Vec<String> {
    let mut out: Vec<String> = eigene_adressen
        .iter()
        .filter_map(|a| eindeutige_identitaet(a))
        .filter_map(|p| p.split_once('@').map(|(_, d)| d.to_string()))
        .filter_map(|d| domaene_normalisieren(&d))
        .filter(|d| als_eigene_zulaessig(d))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Gehört `domaene` zu einem Eintrag der eigenen Domänen?
fn ist_eigen(domaene: &str, eigene: &[EigeneDomaene]) -> bool {
    eigene.iter().any(|e| {
        domaene == e.domaene
            || (e.mit_subdomaenen
                && domaene
                    .strip_suffix(e.domaene.as_str())
                    .is_some_and(|rest| rest.len() > 1 && rest.ends_with('.')))
    })
}

/// Identität eines Eintrags der Teilnehmerliste, nur wenn er GENAU eine Adresse trägt:
/// kein `;`, genau ein `@`, höchstens ein Paar `<…>` und danach nichts mehr, ohne Klammern
/// weder `,` noch Leerraum, und die Adresse selbst nur aus ASCII (kein `K` (Kelvin), das
/// beim Kleinschreiben zu `k` würde). Alles andere zählt als Person ohne Adresse.
fn eindeutige_identitaet(t: &str) -> Option<String> {
    let t = t.trim();
    if t.contains(';') || t.matches('@').count() > 1 {
        return None;
    }
    let adresse = match (t.find('<'), t.find('>')) {
        (None, None) => {
            if t.contains(',') || t.chars().any(char::is_whitespace) {
                return None;
            }
            t
        }
        (Some(a), Some(b)) if a < b => {
            let einmal = t.matches('<').count() == 1 && t.matches('>').count() == 1;
            if !einmal || !t[b + 1..].trim().is_empty() {
                return None;
            }
            &t[a + 1..b]
        }
        _ => return None,
    };
    if !adresse.is_ascii() || adresse.contains(',') {
        return None;
    }
    identitaet(t)
}

/// Fingerabdruck (SHA-256, Hex) einer Teilnehmerliste zusammen mit den wirksamen eigenen
/// Domänen: Reihenfolge und Schreibweise egal; jede hinzugekommene oder entfernte Person
/// und jede Änderung der eigenen Domänen ändert ihn. Personen ohne Identität zählen
/// einzeln mit (zwei Unbekannte sind nicht eine). Die Herkunft der Liste (Termin oder
/// Ad-hoc) zählt mit.
pub fn fingerabdruck(
    teilnehmende: &[String],
    eigene_domaenen: &[String],
    liste_aus_termin: bool,
) -> String {
    use sha2::{Digest, Sha256};
    let mut bekannt: Vec<String> = Vec::new();
    let mut unbekannt = 0usize;
    for t in teilnehmende {
        match eindeutige_identitaet(t) {
            Some(p) => bekannt.push(p),
            None => unbekannt += 1,
        }
    }
    bekannt.sort();
    bekannt.dedup();
    let eigene: Vec<String> = eigene_eintraege(eigene_domaenen)
        .into_iter()
        .map(|e| {
            if e.mit_subdomaenen {
                format!("*.{}", e.domaene)
            } else {
                e.domaene
            }
        })
        .collect();
    let mut h = Sha256::new();
    h.update(b"lva-g9-teilnehmende-v2\n");
    h.update(if liste_aus_termin {
        "termin\n"
    } else {
        "adhoc\n"
    });
    for p in &bekannt {
        h.update(p.as_bytes());
        h.update(b"\n");
    }
    h.update(format!("?{unbekannt}\n#").as_bytes());
    for e in &eigene {
        h.update(e.as_bytes());
        h.update(b"\n");
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Stuft eine Besprechung nach ihrer Teilnehmerliste ein (laut Einladung, nicht laut
/// tatsächlicher Anwesenheit). `liste_aus_termin` ist nur wahr, wenn die Liste aus dem
/// Kalendertermin stammt; eine getippte oder Ad-hoc-Liste ist höchstens „unbekannt“.
pub fn einstufen(
    teilnehmende: &[String],
    eigene_domaenen: &[String],
    liste_aus_termin: bool,
) -> EinstufungsErgebnis {
    let eigene = eigene_eintraege(eigene_domaenen);
    let mut externe: Vec<String> = Vec::new();
    let mut ohne_adresse: u32 = 0;
    let mut intern_gesehen = false;
    for t in teilnehmende {
        let adresse = eindeutige_identitaet(t).and_then(|p| {
            let d = p.split_once('@').map(|(_, d)| d.to_string())?;
            domaene_normalisieren(&d).map(|d| (p.clone(), d))
        });
        match adresse {
            None => ohne_adresse = ohne_adresse.saturating_add(1),
            Some((p, d)) => {
                if eigene.is_empty() {
                    // Ohne eigene Domänen ist keine Adresse als intern belegbar.
                    ohne_adresse = ohne_adresse.saturating_add(1);
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
    } else if ohne_adresse > 0 || !intern_gesehen || !liste_aus_termin {
        Einstufung::Unbekannt
    } else {
        Einstufung::Intern
    };
    EinstufungsErgebnis {
        einstufung,
        externe,
        ohne_adresse,
        aus_termin: liste_aus_termin,
        fingerabdruck: fingerabdruck(teilnehmende, eigene_domaenen, liste_aus_termin),
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

/// Wie verlässlich die Teilnehmerliste ist. Belegt vom Aufrufer (Kalenderabgleich), weil
/// diese Logik keine Uhr hat. Nur [`Teilnehmerinfo::AktuellVollstaendig`] lässt Cloud und
/// MCP zu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Teilnehmerinfo {
    /// Liste vollständig geladen und nach dem maßgeblichen Zeitpunkt erfolgreich abgeglichen.
    AktuellVollstaendig,
    /// Liste gekürzt oder nur teilweise geladen.
    Unvollstaendig,
    /// Letzter erfolgreicher Abgleich zu alt oder letzter Abgleich fehlgeschlagen.
    Veraltet,
    /// Kein Nachweis vorhanden (strenger Standard).
    Unbekannt,
}

/// Bindung einer dokumentierten Freigabe an genau eine Runde: Besprechung, Teilnehmerliste
/// (Fingerabdruck aus [`einstufen`]) und Teilnehmer-Revision. Die App speichert diesen Wert
/// beim Erteilen und übergibt ihn in [`Anfrage::freigabe_bindung`].
pub fn freigabe_bindung(besprechung_id: &str, fingerabdruck: &str, revision: u64) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"lva-g9-freigabe-v1\n");
    h.update((besprechung_id.len() as u64).to_le_bytes());
    h.update(besprechung_id.as_bytes());
    h.update((fingerabdruck.len() as u64).to_le_bytes());
    h.update(fingerabdruck.as_bytes());
    h.update(revision.to_le_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Wohin der Inhalt gehen soll.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Quelle {
    Lokal,
    Cloud,
}

/// Über welchen Weg der Inhalt das Modell oder die App verlässt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Pfad {
    /// Fragen im Live-Chat während der Aufnahme.
    LiveChat,
    /// Laufender Stand (Zusammenfassung während der Besprechung).
    Stand,
    /// Hintergrundauswertung (G5) nach oder während der Besprechung.
    Auswertung,
    /// Ausgabe über den lokalen MCP-Server an einen MCP-Client: immer wie Cloud geprüft.
    Mcp,
}

/// Alles, was die App für eine Prüfung zusammenträgt. Ist eine Angabe nicht belegbar, setzt
/// die App den STRENGEN Wert des Feldes; er steht je Feld dabei. Achtung: Bei
/// `kundenprofil_nur_lokal` und `profil_nur_lokal` ist `true` streng, ein unbekanntes Profil
/// zählt als „nur lokal“.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Anfrage {
    /// Kennung der Besprechung (für die Bindung der Freigabe). Leer sperrt.
    pub besprechung_id: String,
    /// Schalter „Assistent in der Besprechung“. Streng: `false`.
    pub assistent_an: bool,
    /// Aufbewahrungsregel für alle Datenarten festgelegt (lva#536). Streng: `false`.
    pub aufbewahrung_festgelegt: bool,
    /// Dokumentierte Freigabe. Streng: `Offen`.
    pub freigabe: Freigabe,
    /// Gespeicherte Bindung der erteilten Freigabe ([`freigabe_bindung`]). Streng: `None`.
    pub freigabe_bindung: Option<String>,
    /// Zähler der App, der bei JEDER beobachteten Änderung der Runde steigt, auch wenn eine
    /// bereits eingeladene Person später eintritt (unveränderte Einladung). Nie sinkend.
    pub teilnehmer_revision: u64,
    pub quelle: Quelle,
    pub pfad: Pfad,
    /// Kundenprofil sperrt Cloud-Quellen. Streng: `true` (auch bei unbekanntem Profil).
    pub kundenprofil_nur_lokal: bool,
    /// Gewähltes Assistentenprofil ist „nur lokal“ (z. B. Personalbezug). Streng: `true`.
    pub profil_nur_lokal: bool,
    /// Anbieter freigegeben und regelkonform (Freigabe je Anbieter, ADR-0028). Streng: `false`.
    pub anbieter_freigegeben: bool,
    /// Cloud für DIESE Besprechung ausdrücklich gewählt. Streng: `false`.
    pub cloud_fuer_besprechung: bool,
    /// Einstellung „Cloud-Modelle in Besprechungen mit Externen“. Streng: `false`.
    pub cloud_bei_externen: bool,
    /// Aktuelle Teilnehmerliste (laut Einladung oder Planung); [`pruefen`] stuft selbst ein.
    pub teilnehmende: Vec<String>,
    /// Die Liste stammt aus dem Kalendertermin (nicht getippt, nicht Ad-hoc). Streng: `false`.
    pub liste_aus_termin: bool,
    /// Aktualität und Vollständigkeit der Liste. Streng: `Unbekannt`.
    pub teilnehmerinfo: Teilnehmerinfo,
    /// Einstellung „Eigene Domänen“ (`example.org` genau, `*.example.org` mit Subdomänen).
    pub eigene_domaenen: Vec<String>,
    /// Gespeicherte Bestätigung DIESER Besprechung. Streng: `None`.
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
    /// Freigabe für eine frühere Runde erteilt (Person hinzu, entfernt, ersetzt oder später
    /// eingetreten): Pause bis zur erneuten Freigabe, auch lokal.
    FreigabeVeraltet,
    KundenprofilNurLokal,
    ProfilNurLokal,
    AnbieterNichtFreigegeben,
    CloudNichtGewaehlt,
    EinstufungUnbestaetigt,
    EinstufungVeraltet,
    TeilnehmendeUnbekannt,
    /// Liste nicht aus einem Termin (Ad-hoc): sperrt die Cloud immer.
    ListeOhneTermin,
    TeilnehmendeExtern,
    TeilnehmerinfoUnvollstaendig,
    TeilnehmerinfoVeraltet,
    TeilnehmerinfoUnbekannt,
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
fn wirksame_einstufung(
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
    let einstufung = einstufen(&a.teilnehmende, &a.eigene_domaenen, a.liste_aus_termin);
    // Die Freigabe gilt nur für die Runde, für die sie erteilt wurde; geprüft VOR der
    // Quellenwahl, damit eine Änderung der Runde auch die lokale Verarbeitung pausiert.
    match a.freigabe {
        Freigabe::Erteilt => {
            let aktuell = freigabe_bindung(
                &a.besprechung_id,
                &einstufung.fingerabdruck,
                a.teilnehmer_revision,
            );
            if a.besprechung_id.is_empty() || a.freigabe_bindung.as_deref() != Some(&aktuell) {
                gruende.push(Sperrgrund::FreigabeVeraltet);
            }
        }
        Freigabe::Offen => gruende.push(Sperrgrund::FreigabeOffen),
        Freigabe::Abgelehnt => gruende.push(Sperrgrund::FreigabeAbgelehnt),
    }
    let wirksam = wirksame_einstufung(&einstufung, a.bestaetigung.as_ref());
    if a.quelle == Quelle::Cloud || a.pfad == Pfad::Mcp {
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
        // Ohne aktuellen, vollständigen Stand trägt keine Einstufung und keine Bestätigung.
        match a.teilnehmerinfo {
            Teilnehmerinfo::AktuellVollstaendig => {}
            Teilnehmerinfo::Unvollstaendig => {
                gruende.push(Sperrgrund::TeilnehmerinfoUnvollstaendig)
            }
            Teilnehmerinfo::Veraltet => gruende.push(Sperrgrund::TeilnehmerinfoVeraltet),
            Teilnehmerinfo::Unbekannt => gruende.push(Sperrgrund::TeilnehmerinfoUnbekannt),
        }
        // Eine Bestätigung muss zur aktuellen Liste passen und darf nicht lockerer sein als
        // die berechnete Einstufung; eine Bestätigung „unbekannt“ sperrt wie eine berechnete.
        let mut unbekannt_bestaetigt = false;
        match &a.bestaetigung {
            None => gruende.push(Sperrgrund::EinstufungUnbestaetigt),
            Some(b) if b.fingerabdruck != einstufung.fingerabdruck => {
                gruende.push(Sperrgrund::EinstufungVeraltet)
            }
            Some(b)
                if b.einstufung == Einstufung::Intern
                    && einstufung.einstufung != Einstufung::Intern =>
            {
                gruende.push(Sperrgrund::EinstufungUnbestaetigt)
            }
            Some(b) => unbekannt_bestaetigt = b.einstufung == Einstufung::Unbekannt,
        }
        // Unbekannte und Ad-hoc sperren immer, auch neben Externen: „Cloud bei Externen“
        // lockert nur, was belegt extern ist.
        if !einstufung.aus_termin {
            gruende.push(Sperrgrund::ListeOhneTermin);
        }
        if einstufung.ohne_adresse > 0
            || unbekannt_bestaetigt
            || (einstufung.einstufung == Einstufung::Unbekannt && einstufung.aus_termin)
        {
            gruende.push(Sperrgrund::TeilnehmendeUnbekannt);
        }
        if wirksam == Einstufung::Extern && !a.cloud_bei_externen {
            gruende.push(Sperrgrund::TeilnehmendeExtern);
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
