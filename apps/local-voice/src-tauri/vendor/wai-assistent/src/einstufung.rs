//! Lokale Einstufung nach Schwierigkeit (ADR-0028 E4.1, E4.3).
//!
//! Regeln: Trifft eine Aeusserung S0 und eine Absicht, gewinnt die hoehere Stufe. S0 nur bei exakter Gleichheit der normalisierten Aeusserung mit einem Muster (optional mit
//! Hoeflichkeitsanhang); alles andere ist mindestens S1. Bei mehreren Treffern gewinnt die hoechste Stufe
//! (unsicher -> hoeher). Kein Treffer: Absicht "allgemein", Stufe S2. Nie Rueckstufung.

use serde::{Deserialize, Serialize};

use crate::regeltabelle::Regeltabelle;

/// Stufe nach Schwierigkeit; die Ordnung ist die Eskalationsordnung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Stufe {
    S0,
    S1,
    S2,
    S3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sprache {
    De,
    En,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Wochentag {
    Montag,
    Dienstag,
    Mittwoch,
    Donnerstag,
    Freitag,
    Samstag,
    Sonntag,
}

impl Wochentag {
    fn name(self, sprache: Sprache) -> &'static str {
        const DE: [&str; 7] = [
            "Montag",
            "Dienstag",
            "Mittwoch",
            "Donnerstag",
            "Freitag",
            "Samstag",
            "Sonntag",
        ];
        const EN: [&str; 7] = [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ];
        let i = self as usize;
        match sprache {
            Sprache::De => DE[i],
            Sprache::En => EN[i],
        }
    }
}

/// Injizierte Uhrzeit (keine Systemuhr im Kern).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Zeitpunkt {
    pub jahr: i32,
    pub monat: u8,
    pub tag: u8,
    pub stunde: u8,
    pub minute: u8,
    pub wochentag: Wochentag,
}

pub trait Uhr {
    fn jetzt(&self) -> Zeitpunkt;
}

/// Feste Uhr fuer Tests und Wiedergabe.
#[derive(Debug, Clone, Copy)]
pub struct FesteUhr(pub Zeitpunkt);

impl Uhr for FesteUhr {
    fn jetzt(&self) -> Zeitpunkt {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Einstufung {
    pub stufe: Stufe,
    pub absicht: String,
    pub ansage: Option<String>,
    pub antwort: Option<String>,
}

/// Eskalation nimmt immer das Maximum, nie eine Rueckstufung.
pub fn eskalieren(alt: Stufe, neu: Stufe) -> Stufe {
    alt.max(neu)
}

/// Kleinschreibung, Satzzeichen (auch im Satz) werden zu Leerraum, Apostrophe vereinheitlicht,
/// Bindestrich und Apostroph nur innerhalb von Woertern, doppelte Leerzeichen entfernt.
pub fn normalisieren(text: &str) -> String {
    tokens(text).join(" ")
}

fn tokens(text: &str) -> Vec<String> {
    let bereinigt: String = text
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            '\u{2019}' | '\u{2018}' | '\u{00B4}' | '`' | '\u{02BC}' => '\'',
            c if c.is_alphanumeric() || c == '\'' || c == '-' => c,
            _ => ' ',
        })
        .collect();
    bereinigt
        .split_whitespace()
        .map(|t| t.trim_matches(|c| c == '\'' || c == '-'))
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Wortform eines Musterworts.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Art {
    Exakt,
    /// `wort*`: Wortanfang.
    Anfang,
    /// `*wort`: Wortende (deutsche Komposita sind rechtskoepfig).
    Ende,
    /// `*wort*`: Teilwort.
    Teil,
    /// Einzelnes `*`: null bis sechs beliebige Woerter.
    Luecke,
}

const MAX_LUECKE: usize = 6;

/// Muster als Wortfolge mit Platzhaltern (`wort*`, `*wort`, `*wort*`, einzelnes `*`).
fn muster_tokens(muster: &str) -> Vec<(String, Art)> {
    let mut aus = Vec::new();
    for stueck in muster.split_whitespace() {
        if stueck == "*" {
            aus.push((String::new(), Art::Luecke));
            continue;
        }
        let (vorn, hinten) = (stueck.starts_with('*'), stueck.ends_with('*'));
        let t = tokens(stueck);
        let n = t.len();
        for (i, w) in t.into_iter().enumerate() {
            let art = match (vorn && i == 0, hinten && i + 1 == n) {
                (true, true) => Art::Teil,
                (true, false) => Art::Ende,
                (false, true) => Art::Anfang,
                (false, false) => Art::Exakt,
            };
            aus.push((w, art));
        }
    }
    aus
}

fn wort_passt(text: &str, (muster, art): &(String, Art)) -> bool {
    match art {
        Art::Exakt => text == muster,
        Art::Anfang => text.starts_with(muster.as_str()),
        Art::Ende => text.ends_with(muster.as_str()),
        Art::Teil => text.contains(muster.as_str()),
        Art::Luecke => true,
    }
}

fn passt_ab(text: &[String], muster: &[(String, Art)]) -> bool {
    let Some((erstes, rest)) = muster.split_first() else {
        return true;
    };
    if erstes.1 == Art::Luecke {
        return (0..=MAX_LUECKE.min(text.len())).any(|k| passt_ab(&text[k..], rest));
    }
    match text.split_first() {
        Some((t, text_rest)) => wort_passt(t, erstes) && passt_ab(text_rest, rest),
        None => false,
    }
}

fn enthaelt(text: &[String], muster: &[(String, Art)]) -> bool {
    muster.iter().any(|m| m.1 != Art::Luecke)
        && (0..text.len()).any(|i| passt_ab(&text[i..], muster))
}

/// Anzahl der Woerter ohne Luecken (Spezifitaet des Musters).
fn spezifitaet(muster: &[(String, Art)]) -> usize {
    muster.iter().filter(|m| m.1 != Art::Luecke).count()
}

const HOEFLICH_DE: [&str; 2] = ["bitte", "danke"];
const HOEFLICH_EN: [&str; 3] = ["please", "thanks", "thank you"];

fn ohne_hoeflichkeit(norm: &str, sprache: Sprache) -> Option<&str> {
    let liste: &[&str] = match sprache {
        Sprache::De => &HOEFLICH_DE,
        Sprache::En => &HOEFLICH_EN,
    };
    liste.iter().find_map(|h| {
        let rest = norm.strip_suffix(h)?;
        rest.strip_suffix(' ')
    })
}

fn s0_treffer<'a>(
    norm: &str,
    sprache: Sprache,
    tabelle: &'a Regeltabelle,
) -> Option<&'a crate::regeltabelle::S0Regel> {
    let gekuerzt = ohne_hoeflichkeit(norm, sprache);
    tabelle.s0.iter().find(|regel| {
        regel.muster.fuer(sprache).iter().any(|m| {
            let m = normalisieren(m);
            m == norm || gekuerzt == Some(m.as_str())
        })
    })
}

const MONATE_DE: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];
const MONATE_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn monat_name(sprache: Sprache, monat: u8) -> &'static str {
    let i = usize::from(monat.clamp(1, 12)) - 1;
    match sprache {
        Sprache::De => MONATE_DE[i],
        Sprache::En => MONATE_EN[i],
    }
}

/// DE "14:05 Uhr", EN "2:05 PM" (0 Uhr = 12:xx AM, 12 Uhr = 12:xx PM).
pub fn uhrzeit_text(sprache: Sprache, z: &Zeitpunkt) -> String {
    match sprache {
        Sprache::De => format!("{}:{:02} Uhr", z.stunde, z.minute),
        Sprache::En => {
            let h12 = match z.stunde % 12 {
                0 => 12,
                h => h,
            };
            let ampm = if z.stunde < 12 { "AM" } else { "PM" };
            format!("{h12}:{:02} {ampm}", z.minute)
        }
    }
}

/// DE "Sonntag, 8. November 2026", EN "Sunday, November 8, 2026".
pub fn datum_text(sprache: Sprache, z: &Zeitpunkt) -> String {
    let wt = z.wochentag.name(sprache);
    let monat = monat_name(sprache, z.monat);
    match sprache {
        Sprache::De => format!("{wt}, {}. {monat} {}", z.tag, z.jahr),
        Sprache::En => format!("{wt}, {monat} {}, {}", z.tag, z.jahr),
    }
}

fn vorlage_fuellen(vorlage: &str, sprache: Sprache, z: &Zeitpunkt) -> String {
    let zeit = uhrzeit_text(sprache, z);
    // Die Vertragsvorlage DE lautet "Es ist {uhrzeit} Uhr.": das "Uhr" nicht doppeln.
    vorlage
        .replace("{uhrzeit} Uhr", &zeit)
        .replace("{uhrzeit}", &zeit)
        .replace("{datum}", &datum_text(sprache, z))
}

pub(crate) fn allgemein_ansage(sprache: Sprache) -> &'static str {
    match sprache {
        Sprache::De => "Ich denke kurz nach.",
        Sprache::En => "Let me think for a moment.",
    }
}

/// Stuft eine Aeusserung ein. `uhr` liefert Uhrzeit und Datum fuer S0-Vorlagen.
pub fn einstufen(
    text: &str,
    sprache: Sprache,
    tabelle: &Regeltabelle,
    uhr: &dyn Uhr,
) -> Einstufung {
    let norm = normalisieren(text);
    let woerter = tokens(text);
    // (Stufe, Musterlaenge) entscheidet; bei Gleichstand gewinnt der frueheste Eintrag.
    let mut bester: Option<(Stufe, usize, usize)> = None;
    for (i, a) in tabelle.absichten.iter().enumerate() {
        let laenge = a
            .muster
            .fuer(sprache)
            .iter()
            .map(|m| muster_tokens(m))
            .filter(|m| enthaelt(&woerter, m))
            .map(|m| spezifitaet(&m))
            .max();
        let Some(laenge) = laenge else { continue };
        let besser = bester.is_none_or(|(s, l, _)| (a.stufe, laenge) > (s, l));
        if besser {
            bester = Some((a.stufe, laenge, i));
        }
    }
    match bester {
        Some((_, _, i)) => {
            let a = &tabelle.absichten[i];
            Einstufung {
                // S0 in absichten ist per pruefen() ausgeschlossen; sicherheitshalber mindestens S1.
                stufe: eskalieren(Stufe::S1, a.stufe),
                absicht: a.absicht.clone(),
                ansage: Some(a.ansage_vorlage.fuer(sprache).to_owned()),
                antwort: None,
            }
        }
        None => s0_oder_allgemein(&norm, sprache, tabelle, uhr),
    }
}

fn s0_oder_allgemein(
    norm: &str,
    sprache: Sprache,
    tabelle: &Regeltabelle,
    uhr: &dyn Uhr,
) -> Einstufung {
    if let Some(regel) = s0_treffer(norm, sprache, tabelle) {
        let antwort = vorlage_fuellen(regel.antwort_vorlage.fuer(sprache), sprache, &uhr.jetzt());
        return Einstufung {
            stufe: Stufe::S0,
            absicht: regel.absicht.als_str().to_owned(),
            ansage: None,
            antwort: Some(antwort),
        };
    }
    Einstufung {
        stufe: Stufe::S2,
        absicht: "allgemein".to_owned(),
        ansage: Some(allgemein_ansage(sprache).to_owned()),
        antwort: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uhr(stunde: u8, minute: u8) -> FesteUhr {
        FesteUhr(Zeitpunkt {
            jahr: 2026,
            monat: 11,
            tag: 8,
            stunde,
            minute,
            wochentag: Wochentag::Sonntag,
        })
    }

    fn std(text: &str, s: Sprache) -> Einstufung {
        einstufen(text, s, &Regeltabelle::standard(), &uhr(14, 5))
    }

    #[test]
    fn uhrzeit_formate() {
        let z = uhr(14, 5).0;
        assert_eq!(uhrzeit_text(Sprache::De, &z), "14:05 Uhr");
        assert_eq!(uhrzeit_text(Sprache::En, &z), "2:05 PM");
        assert_eq!(uhrzeit_text(Sprache::En, &uhr(0, 7).0), "12:07 AM");
        assert_eq!(uhrzeit_text(Sprache::En, &uhr(12, 0).0), "12:00 PM");
        assert_eq!(uhrzeit_text(Sprache::En, &uhr(23, 59).0), "11:59 PM");
        assert_eq!(uhrzeit_text(Sprache::De, &uhr(0, 0).0), "0:00 Uhr");
    }

    #[test]
    fn datum_formate() {
        let z = uhr(1, 1).0;
        assert_eq!(datum_text(Sprache::De, &z), "Sonntag, 8. November 2026");
        assert_eq!(datum_text(Sprache::En, &z), "Sunday, November 8, 2026");
    }

    #[test]
    fn s0_uhrzeit_und_datum_aus_der_uhr() {
        let e = std("Wie spät ist es?", Sprache::De);
        assert_eq!(e.stufe, Stufe::S0);
        assert_eq!(e.absicht, "uhrzeit");
        assert_eq!(e.antwort.as_deref(), Some("Es ist 14:05 Uhr."));
        let e = std("What time is it", Sprache::En);
        assert_eq!(e.antwort.as_deref(), Some("It is 2:05 PM."));
        let e = std("Welches Datum ist heute?", Sprache::De);
        assert_eq!(
            e.antwort.as_deref(),
            Some("Heute ist der Sonntag, 8. November 2026.")
        );
        let e = std("what's the date today", Sprache::En);
        assert_eq!(
            e.antwort.as_deref(),
            Some("Today is Sunday, November 8, 2026.")
        );
    }

    #[test]
    fn s0_mit_hoeflichkeit_und_apostroph() {
        assert_eq!(std("Hallo, bitte!", Sprache::De).stufe, Stufe::S0);
        assert_eq!(std("Stopp danke", Sprache::De).stufe, Stufe::S0);
        assert_eq!(std("  Stop,   please. ", Sprache::En).stufe, Stufe::S0);
        assert_eq!(std("That\u{2019}s enough", Sprache::En).stufe, Stufe::S0);
        assert_eq!(std("Louder, thanks", Sprache::En).absicht, "lauter");
    }

    #[test]
    fn s0_kein_teilstring() {
        for t in [
            "hallo, wie viele Mails habe ich?",
            "ja und dann?",
            "stopp die Recherche",
        ] {
            assert!(std(t, Sprache::De).stufe > Stufe::S0, "{t}");
        }
        assert!(std("hello, what is on my calendar today", Sprache::En).stufe >= Stufe::S1);
    }

    #[test]
    fn hoechste_stufe_gewinnt() {
        // Mail-Frage (S1) plus Entwurf (S3) -> S3.
        let e = std(
            "such die Mail zum Vertrag und schreib eine Antwort auf die Mail",
            Sprache::De,
        );
        assert_eq!(e.stufe, Stufe::S3);
        assert!(e.ansage.is_some() && e.antwort.is_none());
    }

    #[test]
    fn s0_ueberschneidung_hoehere_stufe_gewinnt() {
        use crate::regeltabelle::{Absicht, MusterJeSprache, VorlageJeSprache};
        let mut t = Regeltabelle::vertragsfixture();
        t.absichten.push(Absicht {
            absicht: "stopp-auftrag".into(),
            stufe: Stufe::S3,
            muster: MusterJeSprache {
                de: vec!["stopp".into()],
                en: vec!["stop".into()],
            },
            ansage_vorlage: VorlageJeSprache {
                de: "Auftrag.".into(),
                en: "Job.".into(),
            },
        });
        let e = einstufen("Stopp", Sprache::De, &t, &uhr(9, 0));
        assert_eq!((e.stufe, e.absicht.as_str()), (Stufe::S3, "stopp-auftrag"));
        assert!(e.antwort.is_none() && e.ansage.is_some());
        let e = einstufen("Stop, please", Sprache::En, &t, &uhr(9, 0));
        assert_eq!(e.stufe, Stufe::S3);
        // Ohne Ueberschneidung bleibt S0.
        assert_eq!(
            einstufen("Halt", Sprache::De, &t, &uhr(9, 0)).stufe,
            Stufe::S0
        );
    }

    #[test]
    fn ohne_treffer_allgemein_s2() {
        let e = std("Erzähl mir etwas Unerwartetes", Sprache::De);
        assert_eq!((e.stufe, e.absicht.as_str()), (Stufe::S2, "allgemein"));
        assert_eq!(e.ansage.as_deref(), Some("Ich denke kurz nach."));
        let e = std("tell me something unexpected", Sprache::En);
        assert_eq!(e.ansage.as_deref(), Some("Let me think for a moment."));
    }

    #[test]
    fn fixture_tabelle_funktioniert_auch() {
        let t = Regeltabelle::vertragsfixture();
        let e = einstufen("Habe ich neue Mails?", Sprache::De, &t, &uhr(9, 0));
        assert_eq!((e.stufe, e.absicht.as_str()), (Stufe::S1, "mail-abfragen"));
    }

    #[test]
    fn eskalation_nie_rueckstufung() {
        assert_eq!(eskalieren(Stufe::S2, Stufe::S1), Stufe::S2);
        assert_eq!(eskalieren(Stufe::S1, Stufe::S3), Stufe::S3);
        assert_eq!(eskalieren(Stufe::S0, Stufe::S0), Stufe::S0);
    }

    #[test]
    fn normalisierung() {
        assert_eq!(normalisieren("  Hallo,,  WELT!! "), "hallo welt");
        assert_eq!(normalisieren("I don\u{2019}t know"), "i don't know");
    }

    #[test]
    fn wildcard_muster() {
        let w = tokens("Schick die Mails von gestern");
        assert!(enthaelt(&w, &muster_tokens("mail*")));
        assert!(!enthaelt(&w, &muster_tokens("mail")));
        assert!(enthaelt(&w, &muster_tokens("die mails")));
        assert!(enthaelt(&w, &muster_tokens("*mail*")));
        assert!(enthaelt(
            &tokens("Der Arzttermin"),
            &muster_tokens("*termin")
        ));
        assert!(!enthaelt(
            &tokens("Der Arzttermin"),
            &muster_tokens("*arzt")
        ));
    }

    #[test]
    fn luecken_muster() {
        let w = tokens("Sag den Termin am Mittwoch ab");
        assert!(enthaelt(&w, &muster_tokens("sag * ab")));
        assert!(!enthaelt(&w, &muster_tokens("sag * zu")));
        let weit = tokens("Sag den Termin am nächsten Mittwoch um zehn Uhr dreißig im Büro ab");
        assert!(!enthaelt(&weit, &muster_tokens("sag * ab")));
        assert!(!enthaelt(&w, &muster_tokens("*")));
    }
}
