//! Zuordnung Notizbuch-Seite ↔ Besprechung (Spec §5.2): Kandidaten, Bewertung, Einstufung.
//! Deterministisch, ohne Modellurteil. Automatik nur mit bestandenem Tor (M5).

use std::collections::{BTreeSet, HashSet};

use wai_notizbuch_zugang::{falte, Seite};

use super::begriffe;
use super::zeit::{blockzeit, lage, Blockzeit, Lage, Zeitachse};

/// Ab dieser Punktzahl wird automatisch zugeordnet (nur mit bestandenem Tor).
pub const SCHWELLE_A: u32 = 60;
/// Ab dieser Punktzahl gibt es einen Vorschlag.
pub const SCHWELLE_B: u32 = 35;
/// Ein Automatik-Treffer braucht diesen Vorsprung vor jeder anderen Besprechung (≥ B).
pub const ABSTAND_PARALLEL: u32 = 15;

pub const PUNKTE_LINK: u32 = 60;
pub const PUNKTE_PROJEKT: u32 = 5;
pub const PUNKTE_ZEIT_MAX: u32 = 40;
pub const PUNKTE_BEGRIFF_JE: u32 = 5;
pub const PUNKTE_BEGRIFFE_MAX: u32 = 25;
pub const PUNKTE_TITEL_DATUM: u32 = 3;
pub const PUNKTE_TITEL_TEILNEHMENDE: u32 = 3;
pub const PUNKTE_TITEL_KALENDER: u32 = 4;

const LINK_PRAEFIX: &str = "wai://local-voice/besprechung/";
const MAX_BEGRIFFE_ANZEIGE: usize = 8;

/// Transkriptabschnitt mit Zeit relativ zum Beginn der Aufnahme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Abschnitt {
    pub start_ms: i64,
    pub ende_ms: i64,
    pub text: String,
}

/// Eingabe: die Besprechung (nur Daten, keine Datenbank).
#[derive(Debug, Clone)]
pub struct Besprechung<'a> {
    pub id: &'a str,
    pub titel: &'a str,
    /// Unix-Sekunden.
    pub started_at_s: i64,
    pub duration_ms: i64,
    pub projekt: Option<&'a str>,
    pub teilnehmende: &'a [String],
    pub kalendertitel: Option<&'a str>,
    pub abschnitte: &'a [Abschnitt],
}

impl Besprechung<'_> {
    pub fn zeitachse(&self) -> Zeitachse {
        Zeitachse::neu(self.started_at_s, self.duration_ms)
    }
}

/// Eingabe: eine Seite mit Herkunftsangaben des Aufrufers.
#[derive(Debug, Clone, Copy)]
pub struct Kandidat<'a> {
    pub seite: &'a Seite,
    pub projekt: Option<&'a str>,
    /// Seite stammt aus Local Voice (beweist keine Zugehörigkeit, M5).
    pub herkunft_local_voice: bool,
}

/// Ein Bewertungsmerkmal mit seinen Punkten; dient zugleich als Zuordnungsgrund für die UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grund {
    Link,
    /// Gleiches Projekt oder Herkunft Local Voice.
    Projekt,
    ZeitAnteil {
        im_fenster: usize,
        gesamt: usize,
        punkte: u32,
    },
    Begriffe {
        begriffe: Vec<String>,
        punkte: u32,
    },
    TitelDatum,
    TitelTeilnehmende,
    TitelKalender,
}

impl Grund {
    pub fn punkte(&self) -> u32 {
        match self {
            Grund::Link => PUNKTE_LINK,
            Grund::Projekt => PUNKTE_PROJEKT,
            Grund::ZeitAnteil { punkte, .. } | Grund::Begriffe { punkte, .. } => *punkte,
            Grund::TitelDatum => PUNKTE_TITEL_DATUM,
            Grund::TitelTeilnehmende => PUNKTE_TITEL_TEILNEHMENDE,
            Grund::TitelKalender => PUNKTE_TITEL_KALENDER,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bewertung {
    pub punkte: u32,
    pub gruende: Vec<Grund>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Einstufung {
    Automatisch,
    Vorschlag,
    Nichts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entscheidung {
    pub bewertung: Bewertung,
    pub einstufung: Einstufung,
}

fn blockzeiten(k: &Kandidat) -> Vec<Option<Blockzeit>> {
    k.seite
        .bloecke
        .iter()
        .filter(|b| !b.text.trim().is_empty())
        .map(|b| blockzeit(b, &k.seite.kopf))
        .collect()
}

/// Kandidat, wenn mindestens eine Blockzeit das Fenster schneidet.
pub fn ist_kandidat(k: &Kandidat, achse: &Zeitachse) -> bool {
    blockzeiten(k).iter().flatten().any(|z| achse.im_fenster(z))
}

/// Enthält der Text einen Link auf genau diese Besprechung? Die Kennung muss vollständig
/// übereinstimmen (`…/abc` trifft nicht `…/abcd`).
pub(crate) fn hat_link(text: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    text.match_indices(LINK_PRAEFIX).any(|(i, _)| {
        let rest = &text[i + LINK_PRAEFIX.len()..];
        let ende = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(rest.len());
        &rest[..ende] == id
    })
}

fn titel_grund(besp: &Besprechung, titel: &str) -> Vec<Grund> {
    let mut out = Vec::new();
    let t = falte(titel);
    let achse = besp.zeitachse();
    let datum_treffer = [achse.beginn_ms, achse.ende_ms].iter().any(|&ms| {
        chrono::DateTime::from_timestamp_millis(ms).is_some_and(|d| {
            let d = d.date_naive();
            [
                d.format("%Y-%m-%d").to_string(),
                d.format("%d.%m.%Y").to_string(),
                d.format("%d.%m.%y").to_string(),
            ]
            .iter()
            .any(|f| t.contains(f.as_str()))
        })
    });
    if datum_treffer {
        out.push(Grund::TitelDatum);
    }
    let name_treffer = besp.teilnehmende.iter().any(|n| {
        let n = falte(n.trim());
        let teil = |w: &str| w.chars().count() >= 4 && t.contains(w);
        (n.chars().count() >= 3 && t.contains(&n)) || n.split_whitespace().any(teil)
    });
    if name_treffer {
        out.push(Grund::TitelTeilnehmende);
    }
    let kal = besp
        .kalendertitel
        .map(|k| falte(k.trim()))
        .unwrap_or_default();
    if kal.chars().count() >= 3 && t.contains(&kal) {
        out.push(Grund::TitelKalender);
    }
    out
}

fn zeit_grund(zeiten: &[Option<Blockzeit>], achse: &Zeitachse) -> Option<Grund> {
    let gesamt = zeiten.len();
    if gesamt == 0 {
        return None;
    }
    let (summe, n) = zeiten
        .iter()
        .flatten()
        .filter(|z| achse.im_fenster(z))
        .fold((0u32, 0usize), |(s, n), z| (s + z.guete.gewicht(), n + 1));
    let punkte = PUNKTE_ZEIT_MAX * summe / (100 * gesamt as u32);
    (n > 0 && punkte > 0).then_some(Grund::ZeitAnteil {
        im_fenster: n,
        gesamt,
        punkte,
    })
}

fn begriff_grund(k: &Kandidat, besp: &Besprechung) -> Option<Grund> {
    let achse = besp.zeitachse();
    let mut gemeinsam: BTreeSet<String> = BTreeSet::new();
    for b in k.seite.bloecke.iter().filter(|b| !b.text.trim().is_empty()) {
        let Some(z) = blockzeit(b, &k.seite.kopf) else {
            continue;
        };
        let Lage::Innerhalb(a) = lage(&z, &achse) else {
            continue;
        };
        let von = a.von_ms.div_euclid(60_000) * 60_000;
        let bis = (a.bis_ms.div_euclid(60_000) + 1) * 60_000;
        let bb = begriffe(&b.text);
        if bb.is_empty() {
            continue;
        }
        for s in besp
            .abschnitte
            .iter()
            .filter(|s| s.start_ms <= bis && s.ende_ms >= von)
        {
            let sb = begriffe(&s.text);
            gemeinsam.extend(bb.intersection(&sb).cloned());
        }
    }
    if gemeinsam.is_empty() {
        return None;
    }
    let punkte = (gemeinsam.len() as u32 * PUNKTE_BEGRIFF_JE).min(PUNKTE_BEGRIFFE_MAX);
    Some(Grund::Begriffe {
        begriffe: gemeinsam.into_iter().take(MAX_BEGRIFFE_ANZEIGE).collect(),
        punkte,
    })
}

/// Bewertet eine Seite gegen eine Besprechung (§5.2).
pub fn bewerte(besp: &Besprechung, k: &Kandidat) -> Bewertung {
    let achse = besp.zeitachse();
    let mut gruende = Vec::new();

    let voll_text = std::iter::once(k.seite.text.as_str())
        .chain(k.seite.bloecke.iter().map(|b| b.text.as_str()))
        .any(|t| hat_link(t, besp.id));
    if voll_text {
        gruende.push(Grund::Link);
    }
    let gleiches_projekt = matches!((k.projekt, besp.projekt), (Some(a), Some(b)) if a == b);
    if gleiches_projekt || k.herkunft_local_voice {
        gruende.push(Grund::Projekt);
    }
    gruende.extend(zeit_grund(&blockzeiten(k), &achse));
    gruende.extend(begriff_grund(k, besp));
    gruende.extend(titel_grund(besp, &k.seite.kopf.titel));

    Bewertung {
        punkte: gruende.iter().map(Grund::punkte).sum(),
        gruende,
    }
}

/// Einstufung nach Schwellen. Ohne bestandenes Tor höchstens `Vorschlag`.
pub fn einstufen(punkte: u32, tor_bestanden: bool) -> Einstufung {
    if punkte >= SCHWELLE_A && tor_bestanden {
        Einstufung::Automatisch
    } else if punkte >= SCHWELLE_B {
        Einstufung::Vorschlag
    } else {
        Einstufung::Nichts
    }
}

/// Gesamtentscheidung für Seite und Besprechung. Kein Kandidat (Blockzeiten außerhalb des
/// Fensters) und gelöste Seiten ergeben immer `Nichts`; „gelöst“ wird nie wieder zugeordnet.
pub fn entscheide(
    besp: &Besprechung,
    k: &Kandidat,
    geloest: &HashSet<String>,
    tor_bestanden: bool,
) -> Entscheidung {
    let bewertung = bewerte(besp, k);
    let einstufung = if geloest.contains(&k.seite.kopf.id) || !ist_kandidat(k, &besp.zeitachse()) {
        Einstufung::Nichts
    } else {
        einstufen(bewertung.punkte, tor_bestanden)
    };
    Entscheidung {
        bewertung,
        einstufung,
    }
}

/// Parallele Besprechungen: dieselbe Seite wurde gegen mehrere Besprechungen bewertet
/// (`punkte[i]` gehört zu `einstufungen[i]`). Ein `Automatisch` ohne Vorsprung
/// ([`ABSTAND_PARALLEL`]) vor einer anderen Besprechung mit mindestens B wird `Vorschlag`.
pub fn entschaerfe_parallele(punkte: &[u32], einstufungen: &mut [Einstufung]) {
    let n = punkte.len().min(einstufungen.len());
    for i in 0..n {
        if einstufungen[i] != Einstufung::Automatisch {
            continue;
        }
        let knapp = (0..n)
            .any(|j| j != i && punkte[j] >= SCHWELLE_B && punkte[j] + ABSTAND_PARALLEL > punkte[i]);
        if knapp {
            einstufungen[i] = Einstufung::Vorschlag;
        }
    }
}
