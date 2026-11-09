//! Verlustfreiheit der Zusammenführung (Spec §5.4): Duplikate, Widersprüche, Verlustprüfung.
//! Deterministisch; Text wird nur gefaltet und verglichen.

use std::collections::{BTreeSet, HashMap, HashSet};

use wai_notizbuch_zugang::{falte, Block, Seite};

use super::begriffe;
use super::zeit::blockzeit;

/// Begriffsüberlappung (Jaccard, ohne Zahlen) ab der zwei Blöcke als Paar gelten.
pub const WIDERSPRUCH_UEBERLAPP: f32 = 0.5;
/// Begriffsüberlappung ab der nicht gleiche Blöcke als „ähnlich notiert“ gelten.
pub const AEHNLICH_UEBERLAPP: f32 = 0.6;
/// Über dieser Blockzahl wird der paarweise Ähnlichkeitsvergleich ausgelassen.
const MAX_BLOECKE_PAARWEISE: usize = 1_500;

/// Herkunft eines Prüfblocks (§5.4 M8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quelle {
    Notizblock,
    Notizbuch,
    Transkript,
}

impl Quelle {
    /// Zulässige Widerspruchspaare: Notizblock ↔ Notizbuch und Notiz (Notizblock oder
    /// Notizbuch) ↔ Transkript. Nie zwei Blöcke derselben Quelle.
    pub fn paar_zulaessig(self, andere: Quelle) -> bool {
        match (self, andere) {
            (Quelle::Notizblock, Quelle::Notizbuch) | (Quelle::Notizbuch, Quelle::Notizblock) => {
                true
            }
            (Quelle::Transkript, Quelle::Transkript) => false,
            (Quelle::Transkript, _) | (_, Quelle::Transkript) => true,
            _ => false,
        }
    }
}

/// Ein Block aus einer der Quellen (Notizblock der Aufnahme, Notizbuch oder Transkript).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruefBlock {
    pub quelle: Quelle,
    /// Stabile Kennung (`seite_id#bid`, siehe [`block_kennung`]) bzw. Notizblock-ID.
    pub kennung: String,
    pub text: String,
    /// Aufgabenstand (`Some` nur bei Aufgaben).
    pub erledigt: Option<bool>,
    /// Zeitintervall (absolute ms, gleiche Einheit in allen Blöcken); `None` = unbekannt.
    pub intervall: Option<(i64, i64)>,
}

/// Kennung eines Notizbuch-Blocks: `seite_id#bid`; ohne `bid` (Altseite) als Notbehelf
/// `seite_id#pos<Index>`.
pub fn block_kennung(seite_id: &str, index: usize, block: &Block) -> String {
    match block.bid.as_deref().filter(|b| !b.is_empty()) {
        Some(bid) => format!("{seite_id}#{bid}"),
        None => format!("{seite_id}#pos{index}"),
    }
}

/// Prüfblöcke einer Seite (leere Blöcke entfallen). Kommt eine `bid` auf der Seite mehrfach
/// vor, behält der erste Block sie; jeder weitere bekommt `seite_id#bid#pos<i>` (i =
/// Blockposition), damit die Verlustprüfung keinen Block hinter einer Doppelkennung verbirgt.
pub fn pruefbloecke_aus_seite(seite: &Seite) -> Vec<PruefBlock> {
    let mut gesehen: HashSet<&str> = HashSet::new();
    seite
        .bloecke
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.text.trim().is_empty())
        .map(|(i, b)| PruefBlock {
            quelle: Quelle::Notizbuch,
            kennung: match b.bid.as_deref().filter(|x| !x.is_empty()) {
                Some(bid) if !gesehen.insert(bid) => {
                    format!("{}#{bid}#pos{i}", seite.kopf.id)
                }
                _ => block_kennung(&seite.kopf.id, i, b),
            },
            text: b.text.clone(),
            erledigt: b.erledigt,
            intervall: blockzeit(b, &seite.kopf).map(|z| (z.frueh_ms, z.spaet_ms)),
        })
        .collect()
}

/// Vergleichsmerkmale eines Blocks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Merkmale {
    pub zahlen: BTreeSet<String>,
    /// Datums- und Uhrzeitangaben, normalisiert.
    pub zeiten: BTreeSet<String>,
    pub verneinungen: usize,
    pub aufgabe: Option<bool>,
}

fn ist_verneinung(w: &str) -> bool {
    matches!(w, "nicht" | "no" | "not") || w.starts_with("kein")
}

/// Englische Kurzform mit „n't“ (don't, isn't, won't …), auch mit typografischem Apostroph.
fn ist_nt_verneinung(w: &str) -> bool {
    let w = w.trim_matches(|c: char| !c.is_alphanumeric());
    w.len() > 3 && (w.ends_with("n't") || w.ends_with("n’t"))
}

fn nur_ziffern(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn datum(w: &str) -> Option<String> {
    let w = w.trim_end_matches([',', ';']);
    let norm = |y: Option<&str>, m: &str, d: &str| -> Option<String> {
        if !(nur_ziffern(m) && nur_ziffern(d)) {
            return None;
        }
        let (m, d): (u32, u32) = (m.parse().ok()?, d.parse().ok()?);
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        let j = match y {
            None => 0,
            Some(y) if nur_ziffern(y) && (y.len() == 2 || y.len() == 4) => {
                let j: u32 = y.parse().ok()?;
                if y.len() == 2 {
                    2000 + j
                } else {
                    j
                }
            }
            _ => return None,
        };
        Some(format!("{j:04}-{m:02}-{d:02}"))
    };
    let p: Vec<&str> = w.split('-').collect();
    if p.len() == 3 && p[0].len() == 4 {
        return norm(Some(p[0]), p[1], p[2]);
    }
    let p: Vec<&str> = w.split('/').collect();
    match p.as_slice() {
        [d, m] => return norm(None, m, d),
        [d, m, y] => return norm(Some(y), m, d),
        _ => {}
    }
    let p: Vec<&str> = w.split('.').collect();
    match p.as_slice() {
        [d, m, ""] => norm(None, m, d),
        [d, m, y] => norm(Some(y), m, d),
        [d, m, y, ""] => norm(Some(y), m, d),
        _ => None,
    }
}

fn uhrzeit(h: &str, m: &str) -> Option<String> {
    if !(nur_ziffern(h) && nur_ziffern(m)) {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

const VORZEICHEN: &[char] = &['-', '+', '−', '<', '>', '=', '≤', '≥', '~'];
const EINHEITEN: &[char] = &['%', '€', '$', '£'];

fn nur_zeichen_aus(w: &str, menge: &[char]) -> bool {
    !w.is_empty() && w.chars().all(|c| menge.contains(&c))
}

/// Ziffernfolgen mit innerem `.`/`,` als Zahlen (`,` wird zu `.`). Vorzeichen bzw.
/// Vergleichszeichen davor und Prozent-/Währungszeichen davor oder dahinter (auch als eigenes
/// Nachbarwort `prev`/`next`) gehören zum Merkmal: `-5`, `5%`, `5€`, `<5` sind verschieden.
fn zahlen_in(w: &str, prev: Option<&str>, next: Option<&str>) -> Vec<String> {
    let zeichen: Vec<char> = w.chars().collect();
    let mut spannen: Vec<(usize, usize, String)> = Vec::new();
    let mut akt = String::new();
    let mut start = 0;
    for (i, &c) in zeichen.iter().enumerate() {
        let trenner_innen = (c == '.' || c == ',')
            && !akt.is_empty()
            && zeichen.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        if c.is_ascii_digit() {
            if akt.is_empty() {
                start = i;
            }
            akt.push(c);
        } else if trenner_innen {
            akt.push('.');
        } else if !akt.is_empty() {
            spannen.push((start, i, std::mem::take(&mut akt)));
        }
    }
    if !akt.is_empty() {
        spannen.push((start, zeichen.len(), akt));
    }
    spannen
        .into_iter()
        .map(|(von, bis, zahl)| {
            let mut vor = String::new();
            let mut k = von;
            while k > 0 && VORZEICHEN.contains(&zeichen[k - 1]) {
                k -= 1;
                vor.insert(0, zeichen[k]);
            }
            // Ein Bindestrich nach einem Buchstaben oder einer Ziffer ist kein Vorzeichen.
            if k > 0 && zeichen[k - 1].is_alphanumeric() {
                vor.retain(|c| c != '-' && c != '−');
            }
            if vor.is_empty() && von == 0 {
                if let Some(p) = prev.filter(|p| nur_zeichen_aus(p, VORZEICHEN)) {
                    vor = p.to_owned();
                }
            }
            let mut einheit = String::new();
            if let Some(&e) = zeichen.get(bis).filter(|c| EINHEITEN.contains(c)) {
                einheit.push(e);
            } else if von > 0 && EINHEITEN.contains(&zeichen[von - 1]) {
                einheit.push(zeichen[von - 1]);
            } else if bis == zeichen.len() {
                if let Some(n) =
                    next.filter(|n| nur_zeichen_aus(n, EINHEITEN) && n.chars().count() == 1)
                {
                    einheit = n.to_owned();
                }
            }
            format!("{vor}{zahl}{einheit}")
        })
        .collect()
}

/// Ermittelt die Vergleichsmerkmale eines Textes.
pub fn merkmale(text: &str, erledigt: Option<bool>) -> Merkmale {
    let gefaltet = falte(text);
    let woerter: Vec<&str> = gefaltet.split_whitespace().collect();
    let mut m = Merkmale {
        aufgabe: erledigt,
        ..Default::default()
    };
    m.verneinungen = gefaltet
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| ist_verneinung(w))
        .count()
        + woerter.iter().filter(|w| ist_nt_verneinung(w)).count();
    let trimm = |s: &str| s.trim_matches(|c: char| !c.is_alphanumeric()).to_owned();
    for (i, roh) in woerter.iter().enumerate() {
        let w = roh.trim_matches(|c: char| {
            matches!(
                c,
                '(' | ')' | '"' | '\'' | ';' | ':' | '!' | '?' | '[' | ']'
            )
        });
        if !w.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Some(d) = datum(w) {
            m.zeiten.insert(d);
            continue;
        }
        let w = w.trim_end_matches([',', '.']);
        if let Some((h, mi)) = w.split_once(':') {
            if let Some(z) = uhrzeit(h, mi) {
                m.zeiten.insert(z);
                continue;
            }
        }
        let uhr_folgt = woerter
            .get(i + 1)
            .map(|n| trimm(n))
            .is_some_and(|n| matches!(n.as_str(), "uhr" | "h" | "oclock"));
        if uhr_folgt {
            let (h, mi) = w.split_once(['.', ',']).unwrap_or((w, "00"));
            if let Some(z) = uhrzeit(h, mi) {
                m.zeiten.insert(z);
                continue;
            }
        }
        let nachbar = |j: Option<usize>| {
            j.and_then(|j| woerter.get(j)).map(|n| {
                n.trim_matches(|c: char| matches!(c, '(' | ')' | '"' | ';' | ':' | '!' | '?'))
            })
        };
        m.zahlen.extend(zahlen_in(
            w,
            nachbar(i.checked_sub(1)),
            nachbar(Some(i + 1)),
        ));
    }
    m
}

/// Schlüssel für „gleicher gefalteter Text“: nur Leerzeichen normalisiert. Satz- und
/// Randzeichen (`-`, `%`, `<` …) bleiben erhalten, sie können die Bedeutung ändern. Ein Text
/// ohne Buchstaben und Ziffern ergibt keinen Schlüssel.
fn textschluessel(text: &str) -> String {
    let g = falte(text);
    if !g.chars().any(char::is_alphanumeric) {
        return String::new();
    }
    g.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beziehung {
    /// Gleicher gefalteter Text und gleiche Zahlen, Zeiten, Verneinungen, Aufgabenstand.
    Duplikat,
    /// Ähnlich, aber nicht gleich: bleibt getrennt, Kennzeichen „ähnlich notiert“.
    Aehnlich,
}

/// Paar von Blockindizes (`a < b`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paar {
    pub a: usize,
    pub b: usize,
    pub beziehung: Beziehung,
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    let u = a.union(b).count();
    if u == 0 {
        return 0.0;
    }
    a.intersection(b).count() as f32 / u as f32
}

fn begriffe_ohne_zahlen(text: &str) -> BTreeSet<String> {
    begriffe(text)
        .into_iter()
        .filter(|w| !w.chars().any(|c| c.is_ascii_digit()))
        .collect()
}

/// Findet Duplikate und ähnliche Blöcke. Ein Block gilt nur als Duplikat, wenn Text und alle
/// Merkmale übereinstimmen; sonst höchstens „ähnlich“.
pub fn finde_duplikate(bloecke: &[PruefBlock]) -> Vec<Paar> {
    let feat: Vec<Merkmale> = bloecke
        .iter()
        .map(|b| merkmale(&b.text, b.erledigt))
        .collect();
    let schl: Vec<String> = bloecke.iter().map(|b| textschluessel(&b.text)).collect();
    let mut gruppen: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, s) in schl.iter().enumerate().filter(|(_, s)| !s.is_empty()) {
        gruppen.entry(s.as_str()).or_default().push(i);
    }
    let mut out = Vec::new();
    let mut gleichtext: HashSet<(usize, usize)> = HashSet::new();
    for g in gruppen.values() {
        for (x, &a) in g.iter().enumerate() {
            for &b in &g[x + 1..] {
                gleichtext.insert((a, b));
                let beziehung = if feat[a] == feat[b] {
                    Beziehung::Duplikat
                } else {
                    Beziehung::Aehnlich
                };
                out.push(Paar { a, b, beziehung });
            }
        }
    }
    if bloecke.len() <= MAX_BLOECKE_PAARWEISE {
        let terme: Vec<BTreeSet<String>> = bloecke.iter().map(|b| begriffe(&b.text)).collect();
        for a in 0..bloecke.len() {
            for b in a + 1..bloecke.len() {
                if gleichtext.contains(&(a, b)) || terme[a].is_empty() || schl[a].is_empty() {
                    continue;
                }
                if jaccard(&terme[a], &terme[b]) >= AEHNLICH_UEBERLAPP {
                    out.push(Paar {
                        a,
                        b,
                        beziehung: Beziehung::Aehnlich,
                    });
                }
            }
        }
    }
    out.sort_by_key(|p| (p.a, p.b));
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Abweichung {
    Zahlen,
    DatumUhrzeit,
    Verneinung,
    Aufgabenstand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widerspruch {
    pub a: usize,
    pub b: usize,
    pub abweichungen: Vec<Abweichung>,
}

/// Mengen weichen ab, wenn beide gefüllt sind und keine die andere umfasst
/// (`{5}` gegen `{5, 3}` ist Ergänzung, kein Widerspruch).
fn mengen_weichen_ab(a: &BTreeSet<String>, b: &BTreeSet<String>) -> bool {
    !a.is_empty() && !b.is_empty() && !a.is_subset(b) && !b.is_subset(a)
}

fn intervalle_ueberlappen(a: Option<(i64, i64)>, b: Option<(i64, i64)>) -> bool {
    match (a, b) {
        (Some((a0, a1)), Some((b0, b1))) => a0 <= b1 && b0 <= a1,
        _ => false,
    }
}

/// Widersprüche nach §5.4 (M8): nur zwischen zulässigen Quellenpaaren (Notizblock ↔ Notizbuch,
/// Notiz ↔ Transkript, nie dieselbe Quelle). Blockpaare mit Begriffsüberlappung ≥ [`WIDERSPRUCH_UEBERLAPP`] im
/// gleichen Zeitintervall, die sich in Zahlen, Datum/Uhrzeit, Verneinung oder Aufgabenstand
/// unterscheiden. Andere Widersprüche werden nicht behauptet. Blöcke ohne Intervall werden
/// nicht verglichen.
pub fn finde_widersprueche(bloecke: &[PruefBlock]) -> Vec<Widerspruch> {
    let feat: Vec<Merkmale> = bloecke
        .iter()
        .map(|b| merkmale(&b.text, b.erledigt))
        .collect();
    let terme: Vec<BTreeSet<String>> = bloecke
        .iter()
        .map(|b| begriffe_ohne_zahlen(&b.text))
        .collect();
    let mut out = Vec::new();
    for a in 0..bloecke.len() {
        if terme[a].is_empty() {
            continue;
        }
        for b in a + 1..bloecke.len() {
            if !bloecke[a].quelle.paar_zulaessig(bloecke[b].quelle)
                || !intervalle_ueberlappen(bloecke[a].intervall, bloecke[b].intervall)
                || jaccard(&terme[a], &terme[b]) < WIDERSPRUCH_UEBERLAPP
            {
                continue;
            }
            let (fa, fb) = (&feat[a], &feat[b]);
            let mut ab = Vec::new();
            if mengen_weichen_ab(&fa.zahlen, &fb.zahlen) {
                ab.push(Abweichung::Zahlen);
            }
            if mengen_weichen_ab(&fa.zeiten, &fb.zeiten) {
                ab.push(Abweichung::DatumUhrzeit);
            }
            if (fa.verneinungen > 0) != (fb.verneinungen > 0) {
                ab.push(Abweichung::Verneinung);
            }
            if matches!((fa.aufgabe, fb.aufgabe), (Some(x), Some(y)) if x != y) {
                ab.push(Abweichung::Aufgabenstand);
            }
            if !ab.is_empty() {
                out.push(Widerspruch {
                    a,
                    b,
                    abweichungen: ab,
                });
            }
        }
    }
    out
}

/// Verlustprüfung: gibt die erwarteten Block-Kennungen zurück, die weder in den
/// Ergebnis-Referenzen noch in „Weitere Notizen“ stehen (Reihenfolge der Eingabe, ohne
/// Doppelte). Der Aufrufer hängt sie an „Weitere Notizen“ an; nichts wird verworfen.
pub fn fehlende_bloecke(
    erwartet: &[String],
    referenzen: &[String],
    weitere_notizen: &[String],
) -> Vec<String> {
    let da: HashSet<&str> = referenzen
        .iter()
        .chain(weitere_notizen)
        .map(String::as_str)
        .collect();
    let mut gesehen: HashSet<&str> = HashSet::new();
    erwartet
        .iter()
        .filter(|k| !k.is_empty() && !da.contains(k.as_str()) && gesehen.insert(k.as_str()))
        .cloned()
        .collect()
}

/// Seiten, die nicht vollständig gelesen wurden („Notizen unvollständig: Seite“).
pub fn unvollstaendige_seiten(seiten: &[&Seite]) -> Vec<String> {
    seiten
        .iter()
        .filter(|s| s.unvollstaendig)
        .map(|s| s.kopf.id.clone())
        .collect()
}
