//! Reine Zusammenführ-Logik der Dokumentenablage (ADR-0029 E4, Spec §2.1/§2.2). Keine I/O, keine Uhr.
//!
//! * Register `{wert, zaehler, geraet}`: das höhere Paar `(zaehler, geraet)` gewinnt, deterministisch.
//! * Fassungen werden nach `fas` vereinigt (nie nach Blob), ohne die IDs in `entfernt`; `entfernt`,
//!   `wahlen` und `texte` wachsen nur.
//! * Wahlen sind unveränderliche Ereignisse; aktiv ist, was in keinem `ersetzt` steht. Abgelöst ist F,
//!   wenn eine aktive Wahl W (F in `gegen`, F nicht gewählt) existiert und keine aktive Wahl mit höherem
//!   Rang als W die Fassung F gewählt hat. Köpfe = ohne Nachfolger und nicht abgelöst; mehr als ein Kopf
//!   ist ein Konflikt.
//! * Ordner: Register werden nie wegen eines Zyklus verändert. Die wirksame Elternzuordnung wird je Stand
//!   abgeleitet ([`wirksame_eltern`]).
//!
//! Jedes Ergebnis ist unabhängig von der Reihenfolge der Zusammenführung (Tests mit Permutationen).

// D4 (Sync) und D3 (KI) nutzen diese Funktionen; D1 liefert Logik und Tests (Spec §9).
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Ein Wert mit Lamport-Zähler und Gerät: das höhere `(zaehler, geraet)` gewinnt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Register<T> {
    pub wert: T,
    pub zaehler: u64,
    pub geraet: String,
}

/// Ordnung: erst `(zaehler, geraet)`, bei Gleichstand der Wert (Totalordnung, nie der Wert vor dem Rang).
impl<T: Ord> Ord for Register<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rang()
            .cmp(&other.rang())
            .then_with(|| self.wert.cmp(&other.wert))
    }
}

impl<T: Ord> PartialOrd for Register<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Register<T> {
    pub fn neu(wert: T, zaehler: u64, geraet: &str) -> Self {
        Self {
            wert,
            zaehler,
            geraet: geraet.to_string(),
        }
    }

    pub fn rang(&self) -> (u64, &str) {
        (self.zaehler, self.geraet.as_str())
    }
}

/// Das gewinnende Register. Bei gleichem Rang entscheidet der Wert (nur für die Totalordnung; ein Gerät
/// vergibt jeden Zähler höchstens einmal).
pub fn gewinner<T: Ord + Clone>(a: &Register<T>, b: &Register<T>) -> Register<T> {
    if a >= b {
        a.clone()
    } else {
        b.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Text {
    pub textrev: String,
    pub blob: String,
    pub size: u64,
    pub extraktor: String,
    pub extraktor_version: String,
    pub marken_version: u32,
    /// `ok`, `kein_text` oder `fehler`.
    pub stand: String,
    /// Fehlercode bei `fehler` (nie Inhalt), z. B. `extrakt.zeit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Bisherige Versuche bei einem vorübergehenden Fehler (siehe `textfassung::wiederholbar`); beim
    /// Zusammenführen gewinnt der höhere Zähler.
    #[serde(default, skip_serializing_if = "ist_null")]
    pub versuche: u32,
    /// Bisherige Zeitüberschreitungen (`extrakt.zeit`), eigener Zähler: genau ein Wiederholversuch mit
    /// größerer Zeitgrenze, unabhängig von anderen vorübergehenden Fehlern.
    #[serde(default, skip_serializing_if = "ist_null")]
    pub versuche_zeit: u32,
}

fn ist_null(n: &u32) -> bool {
    *n == 0
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Fassung {
    pub fas: String,
    pub vorgaenger: Option<String>,
    pub blob: String,
    pub size: u64,
    pub original_sha256: String,
    pub typ: String,
    pub geraet: String,
    pub zeit: i64,
    #[serde(default)]
    pub texte: Vec<Text>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Wahl {
    pub id: String,
    pub gewaehlt: String,
    #[serde(default)]
    pub gegen: Vec<String>,
    #[serde(default)]
    pub ersetzt: Vec<String>,
    pub zaehler: u64,
    pub geraet: String,
}

impl Wahl {
    fn rang(&self) -> (u64, &str) {
        (self.zaehler, self.geraet.as_str())
    }
}

/// Datensatz `ablage_datei` (Spec §2.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Datei {
    pub id: String,
    pub ordner: Register<String>,
    pub name: Register<String>,
    pub klasse_min: Register<String>,
    #[serde(default)]
    pub nur_geraet_min: bool,
    #[serde(default)]
    pub fassungen: Vec<Fassung>,
    #[serde(default)]
    pub wahlen: Vec<Wahl>,
    #[serde(default)]
    pub entfernt: Vec<String>,
    #[serde(default)]
    pub blob_refs: Vec<String>,
    pub papierkorb: Register<Option<i64>>,
    /// Felder einer neueren Fassung des Datensatzes: bleiben erhalten und reisen mit (D4).
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Datensatz `ablage_ordner` (Spec §2.2). Kein `ki`-Feld: Freigaben sind gerätelokal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ordner {
    pub id: String,
    pub eltern: Register<Option<String>>,
    pub name: Register<String>,
    pub datenklasse: Register<String>,
    pub nur_geraet: Register<bool>,
    pub zielordner_ki: Register<bool>,
    pub papierkorb: Register<Option<i64>>,
    /// Felder einer neueren Fassung des Datensatzes: bleiben erhalten und reisen mit (D4).
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Unbekannte Felder vereinigen; bei gleichem Schlüssel gewinnt der (als JSON) größere Wert, damit das
/// Ergebnis nicht von der Reihenfolge abhängt.
pub fn extra_vereinigen(
    a: &BTreeMap<String, serde_json::Value>,
    b: &BTreeMap<String, serde_json::Value>,
) -> BTreeMap<String, serde_json::Value> {
    let mut aus = a.clone();
    for (k, v) in b {
        match aus.get(k) {
            Some(alt) if alt.to_string() >= v.to_string() => {}
            _ => {
                aus.insert(k.clone(), v.clone());
            }
        }
    }
    aus
}

/// Blob-Kennungen aller lebenden Fassungen und Texte (sichtbares Steuerfeld), sortiert und eindeutig.
pub fn blob_refs_aus(fassungen: &[Fassung]) -> Vec<String> {
    let mut m: BTreeSet<String> = BTreeSet::new();
    for f in fassungen {
        m.insert(f.blob.clone());
        // Ein fehlgeschlagener Lauf hat keinen Blob (leere Kennung).
        for t in f.texte.iter().filter(|t| !t.blob.is_empty()) {
            m.insert(t.blob.clone());
        }
    }
    m.into_iter().collect()
}

fn vereinigt<T: Ord + Clone>(a: &[T], b: &[T]) -> Vec<T> {
    a.iter()
        .chain(b.iter())
        .cloned()
        .collect::<BTreeSet<T>>()
        .into_iter()
        .collect()
}

/// Fassung gleicher `fas`: unveränderliche Felder bleiben, `texte` werden vereinigt (nach `textrev`).
fn fassung_vereinigen(a: &Fassung, b: &Fassung) -> Fassung {
    let mut basis = if a >= b { a.clone() } else { b.clone() };
    let mut je_rev: BTreeMap<String, Text> = BTreeMap::new();
    for t in a.texte.iter().chain(b.texte.iter()) {
        let e = je_rev.entry(t.textrev.clone()).or_insert_with(|| t.clone());
        if t > e {
            *e = t.clone();
        }
    }
    basis.texte = je_rev.into_values().collect();
    basis
}

/// Führt zwei Stände derselben Datei zusammen (kommutativ, assoziativ, idempotent).
pub fn zusammenfuehren_datei(a: &Datei, b: &Datei) -> Datei {
    let entfernt = vereinigt(&a.entfernt, &b.entfernt);
    let weg: BTreeSet<&String> = entfernt.iter().collect();
    let mut je_fas: BTreeMap<String, Fassung> = BTreeMap::new();
    for f in a.fassungen.iter().chain(b.fassungen.iter()) {
        if weg.contains(&f.fas) {
            continue;
        }
        match je_fas.get(&f.fas) {
            Some(alt) => {
                let neu = fassung_vereinigen(alt, f);
                je_fas.insert(f.fas.clone(), neu);
            }
            None => {
                je_fas.insert(f.fas.clone(), f.clone());
            }
        }
    }
    let fassungen: Vec<Fassung> = je_fas.into_values().collect();
    let mut je_wahl: BTreeMap<String, Wahl> = BTreeMap::new();
    for w in a.wahlen.iter().chain(b.wahlen.iter()) {
        let e = je_wahl.entry(w.id.clone()).or_insert_with(|| w.clone());
        if w > e {
            *e = w.clone();
        }
    }
    Datei {
        id: a.id.clone(),
        ordner: gewinner(&a.ordner, &b.ordner),
        name: gewinner(&a.name, &b.name),
        klasse_min: gewinner(&a.klasse_min, &b.klasse_min),
        nur_geraet_min: a.nur_geraet_min || b.nur_geraet_min,
        blob_refs: blob_refs_aus(&fassungen),
        fassungen,
        wahlen: je_wahl.into_values().collect(),
        entfernt,
        papierkorb: gewinner(&a.papierkorb, &b.papierkorb),
        extra: extra_vereinigen(&a.extra, &b.extra),
    }
}

/// Führt zwei Stände desselben Ordners zusammen.
pub fn zusammenfuehren_ordner(a: &Ordner, b: &Ordner) -> Ordner {
    Ordner {
        id: a.id.clone(),
        eltern: gewinner(&a.eltern, &b.eltern),
        name: gewinner(&a.name, &b.name),
        datenklasse: gewinner(&a.datenklasse, &b.datenklasse),
        nur_geraet: gewinner(&a.nur_geraet, &b.nur_geraet),
        zielordner_ki: gewinner(&a.zielordner_ki, &b.zielordner_ki),
        papierkorb: gewinner(&a.papierkorb, &b.papierkorb),
        extra: extra_vereinigen(&a.extra, &b.extra),
    }
}

/// Führt zwei Ordnerbestände zusammen (Vereinigung nach ID, gleiche IDs per Register).
pub fn zusammenfuehren_ordnerbestand(
    a: &BTreeMap<String, Ordner>,
    b: &BTreeMap<String, Ordner>,
) -> BTreeMap<String, Ordner> {
    let mut aus = a.clone();
    for (id, o) in b {
        let neu = match aus.get(id) {
            Some(alt) => zusammenfuehren_ordner(alt, o),
            None => o.clone(),
        };
        aus.insert(id.clone(), neu);
    }
    aus
}

// ---------------------------------------------------------------------------
// Wahlen, Köpfe, Konflikt
// ---------------------------------------------------------------------------

/// Aktive Wahlen: in keinem `ersetzt` einer Wahl genannt.
pub fn aktive_wahlen(d: &Datei) -> Vec<&Wahl> {
    let ersetzt: BTreeSet<&str> = d
        .wahlen
        .iter()
        .flat_map(|w| w.ersetzt.iter().map(String::as_str))
        .collect();
    d.wahlen
        .iter()
        .filter(|w| !ersetzt.contains(w.id.as_str()))
        .collect()
}

/// Abgelöste Fassungen (Vorrangregel, ADR-0029 E4).
pub fn abgeloest(d: &Datei) -> BTreeSet<String> {
    let aktiv = aktive_wahlen(d);
    let mut aus = BTreeSet::new();
    for w in &aktiv {
        for f in &w.gegen {
            if *f == w.gewaehlt {
                continue;
            }
            let ueberstimmt = aktiv
                .iter()
                .any(|h| h.rang() > w.rang() && h.gewaehlt == *f);
            if !ueberstimmt {
                aus.insert(f.clone());
            }
        }
    }
    aus
}

/// Köpfe: lebende Fassungen ohne Nachfolger, die nicht abgelöst sind (sortiert nach `fas`).
pub fn koepfe(d: &Datei) -> Vec<String> {
    let mit_nachfolger: BTreeSet<&str> = d
        .fassungen
        .iter()
        .filter_map(|f| f.vorgaenger.as_deref())
        .collect();
    let abgel = abgeloest(d);
    let mut k: Vec<String> = d
        .fassungen
        .iter()
        .filter(|f| !mit_nachfolger.contains(f.fas.as_str()) && !abgel.contains(&f.fas))
        .map(|f| f.fas.clone())
        .collect();
    k.sort();
    k
}

/// Mehr als ein Kopf.
pub fn konflikt(d: &Datei) -> bool {
    koepfe(d).len() > 1
}

/// Eine Fassung und alle ihre Vorgängerinnen (nur lebende), nächste zuerst.
fn ahnen<'a>(d: &'a Datei, fas: &str) -> Vec<&'a str> {
    let je: BTreeMap<&str, &Fassung> = d.fassungen.iter().map(|f| (f.fas.as_str(), f)).collect();
    let mut aus = Vec::new();
    let mut aktuell = je.get(fas).and_then(|f| f.vorgaenger.as_deref());
    let mut schritte = 0usize;
    while let Some(id) = aktuell {
        let Some(f) = je.get(id) else { break };
        aus.push(f.fas.as_str());
        aktuell = f.vorgaenger.as_deref();
        schritte += 1;
        if schritte > d.fassungen.len() {
            break;
        }
    }
    aus
}

/// Jüngste gemeinsame Vorgängerfassung aller offenen Köpfe. `None` bei weniger als zwei Köpfen oder
/// getrennten Wurzeln (dann nutzt die KI die Datei bis zur Wahl nicht).
pub fn gemeinsame_vorgaengerin(d: &Datei) -> Option<String> {
    let k = koepfe(d);
    if k.len() < 2 {
        return None;
    }
    let listen: Vec<Vec<&str>> = k.iter().map(|f| ahnen(d, f)).collect();
    let (erste, rest) = listen.split_first()?;
    erste
        .iter()
        .find(|kandidat| rest.iter().all(|l| l.contains(kandidat)))
        .map(|s| s.to_string())
}

/// Fassung, die „aktuell“ ist: der einzige Kopf, sonst `None` (nie per Zeit gewählt).
pub fn aktuell(d: &Datei) -> Option<String> {
    let mut k = koepfe(d);
    if k.len() == 1 {
        k.pop()
    } else {
        None
    }
}

/// Neue Wahl: `gegen` = offene Köpfe ∪ `gegen` aller ersetzten Wahlen; `ersetzt` = alle aktiven Wahlen,
/// die dieselben Fassungen betreffen (hier: jede aktive Wahl, deren `gegen` oder `gewaehlt` sich mit
/// dem `gegen` der neuen schneidet).
pub fn neue_wahl(d: &Datei, id: &str, gewaehlt: &str, zaehler: u64, geraet: &str) -> Wahl {
    let mut gegen: BTreeSet<String> = koepfe(d).into_iter().collect();
    gegen.insert(gewaehlt.to_string());
    // Betroffen sind die Köpfe, die gewählte Fassung und ihre Vorgängerinnen.
    for f in gegen.clone() {
        gegen.extend(ahnen(d, &f).into_iter().map(str::to_string));
    }
    let betroffen = gegen.clone();
    let mut gegen: BTreeSet<String> = koepfe(d).into_iter().collect();
    gegen.insert(gewaehlt.to_string());
    let mut betroffen = betroffen;
    let mut ersetzt: BTreeSet<String> = BTreeSet::new();
    loop {
        let vorher = (betroffen.len(), ersetzt.len());
        for w in aktive_wahlen(d) {
            let beruehrt =
                w.gegen.iter().any(|f| betroffen.contains(f)) || betroffen.contains(&w.gewaehlt);
            if beruehrt {
                ersetzt.insert(w.id.clone());
                gegen.extend(w.gegen.iter().cloned());
                gegen.insert(w.gewaehlt.clone());
                betroffen.extend(w.gegen.iter().cloned());
                betroffen.insert(w.gewaehlt.clone());
            }
        }
        if (betroffen.len(), ersetzt.len()) == vorher {
            break;
        }
    }
    Wahl {
        id: id.to_string(),
        gewaehlt: gewaehlt.to_string(),
        gegen: gegen.into_iter().collect(),
        ersetzt: ersetzt.into_iter().collect(),
        zaehler,
        geraet: geraet.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Ordnerbaum
// ---------------------------------------------------------------------------

/// Wirksame Elternzuordnung eines Ordners.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Eltern {
    Wurzel,
    Ordner(String),
    /// Zyklus oder unbekannter bzw. begrabener Elternordner: vorläufig unter „Wiederhergestellt“.
    Wiederhergestellt,
}

/// Leitet die wirksame Elternzuordnung ab (die Register bleiben unverändert): Ordner absteigend nach dem
/// Rang ihres `eltern`-Registers; eine Kante gilt nur, wenn sie mit den bereits übernommenen keinen
/// Zyklus bildet.
pub fn wirksame_eltern(ordner: &BTreeMap<String, Ordner>) -> BTreeMap<String, Eltern> {
    let mut reihe: Vec<&Ordner> = ordner.values().collect();
    reihe.sort_by(|a, b| {
        b.eltern
            .rang()
            .cmp(&a.eltern.rang())
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut aus: BTreeMap<String, Eltern> = BTreeMap::new();
    for o in reihe {
        let e = match &o.eltern.wert {
            None => Eltern::Wurzel,
            Some(p) if !ordner.contains_key(p) => Eltern::Wiederhergestellt,
            Some(p) => {
                if fuehrt_zu(&aus, p, &o.id) {
                    Eltern::Wiederhergestellt
                } else {
                    Eltern::Ordner(p.clone())
                }
            }
        };
        aus.insert(o.id.clone(), e);
    }
    aus
}

/// Führt die Kette der übernommenen Kanten von `von` aufwärts zu `ziel` (oder ist `von == ziel`)?
fn fuehrt_zu(angenommen: &BTreeMap<String, Eltern>, von: &str, ziel: &str) -> bool {
    let mut aktuell = von.to_string();
    for _ in 0..=angenommen.len() + 1 {
        if aktuell == ziel {
            return true;
        }
        match angenommen.get(&aktuell) {
            Some(Eltern::Ordner(p)) => aktuell = p.clone(),
            _ => return false,
        }
    }
    true
}

/// Kette eines Ordners von sich selbst bis zur Wurzel (nach der wirksamen Zuordnung, ohne Zyklus).
pub fn kette(eltern: &BTreeMap<String, Eltern>, id: &str) -> Vec<String> {
    let mut aus = vec![id.to_string()];
    let mut aktuell = id.to_string();
    while let Some(Eltern::Ordner(p)) = eltern.get(&aktuell) {
        if aus.contains(p) {
            break;
        }
        aus.push(p.clone());
        aktuell = p.clone();
    }
    aus
}

/// Zusätzliche Vorfahren nur für Einschränkungen: Liegt ein Ordner wirksam unter „Wiederhergestellt“, bleibt
/// die gespeicherte Elternkette für Sperre, Klasse und „nur dieses Gerät“ maßgeblich (streng: Vereinigung
/// beider Ketten, die strengste Regel gewinnt).
pub fn zusatzkette(
    ordner: &BTreeMap<String, Ordner>,
    eltern: &BTreeMap<String, Eltern>,
    id: &str,
) -> Vec<String> {
    let wirksam = kette(eltern, id);
    let mut zusatz: Vec<String> = Vec::new();
    let Some(letzter) = wirksam.last() else {
        return zusatz;
    };
    if !matches!(eltern.get(letzter), Some(Eltern::Wiederhergestellt)) {
        return zusatz;
    }
    let mut aktuell = ordner.get(letzter).and_then(|o| o.eltern.wert.clone());
    while let Some(p) = aktuell {
        if wirksam.contains(&p) || zusatz.contains(&p) {
            break;
        }
        aktuell = ordner.get(&p).and_then(|o| o.eltern.wert.clone());
        zusatz.push(p);
    }
    zusatz
}

/// Wirksame Kette plus [`zusatzkette`].
pub fn strenge_kette(
    ordner: &BTreeMap<String, Ordner>,
    eltern: &BTreeMap<String, Eltern>,
    id: &str,
) -> Vec<String> {
    let mut k = kette(eltern, id);
    k.extend(zusatzkette(ordner, eltern, id));
    k
}

/// Datenklasse eines Ordners oder einer Datei.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Klasse {
    Public,
    Internal,
    Confidential,
    Personal,
    /// Unbekannter Wert nach dem Zusammenführen: wie gesperrt, bis der Nutzer ihn setzt.
    Unbekannt,
}

impl Klasse {
    pub const VORGABE: Klasse = Klasse::Internal;

    pub fn aus_text(t: &str) -> Option<Klasse> {
        match t {
            "public" => Some(Klasse::Public),
            "internal" => Some(Klasse::Internal),
            "confidential" => Some(Klasse::Confidential),
            "personal" => Some(Klasse::Personal),
            _ => None,
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Klasse::Public => "public",
            Klasse::Internal => "internal",
            Klasse::Confidential => "confidential",
            Klasse::Personal => "personal",
            Klasse::Unbekannt => "unbekannt",
        }
    }
}

/// Wirksame Klasse: von der Wurzel abwärts; `erbt` übernimmt, ein bekannter Wert nur gleich oder höher
/// (eine niedrigere gespeicherte Klasse wird auf die geerbte angehoben, nie umgekehrt), ein unbekannter
/// Wert macht das Ergebnis `Unbekannt`.
pub fn wirksame_klasse(
    ordner: &BTreeMap<String, Ordner>,
    eltern: &BTreeMap<String, Eltern>,
    id: &str,
) -> Klasse {
    let mut geerbt = Klasse::VORGABE;
    for glied in strenge_kette(ordner, eltern, id).iter().rev() {
        let Some(o) = ordner.get(glied) else {
            return Klasse::Unbekannt;
        };
        let wert = o.datenklasse.wert.as_str();
        if wert == "erbt" {
            continue;
        }
        match Klasse::aus_text(wert) {
            Some(k) => geerbt = geerbt.max(k),
            None => return Klasse::Unbekannt,
        }
    }
    geerbt
}

/// Klasse einer Datei: Maximum aus der Klasse ihres Ordners und der eigenen Mindestklasse.
pub fn wirksame_klasse_datei(
    ordner: &BTreeMap<String, Ordner>,
    eltern: &BTreeMap<String, Eltern>,
    d: &Datei,
) -> Klasse {
    let ordnerklasse = wirksame_klasse(ordner, eltern, &d.ordner.wert);
    if ordnerklasse == Klasse::Unbekannt {
        return Klasse::Unbekannt;
    }
    match Klasse::aus_text(&d.klasse_min.wert) {
        Some(k) => ordnerklasse.max(k),
        None if d.klasse_min.wert == "erbt" => ordnerklasse,
        None => Klasse::Unbekannt,
    }
}

/// „Nur dieses Gerät“ ist eine Unterbaumregel: wirksam, sobald der Ordner oder ein Vorfahr es trägt.
pub fn wirksam_nur_geraet(
    ordner: &BTreeMap<String, Ordner>,
    eltern: &BTreeMap<String, Eltern>,
    id: &str,
) -> bool {
    strenge_kette(ordner, eltern, id)
        .iter()
        .any(|g| ordner.get(g).is_some_and(|o| o.nur_geraet.wert))
}

/// Obergrenze für Zähler (`ordner_zaehler_ok`, `datei_zaehler_ok`, `naechster_zaehler`): Datensätze
/// darüber gelten beim Sync als ungültig, und lokal wird nie mehr als `ZAEHLER_MAX` vergeben. Ein einzelner
/// Datensatz (beschädigt oder manipuliert) kann die Kette so nicht erschöpfen. Genau `ZAEHLER_MAX` gilt noch;
/// danach ist die Kette ausgeschöpft (`ablage.zaehler_erschoepft`).
pub const ZAEHLER_MAX: u64 = u64::MAX / 2;

/// Größter Zähler in den Registern eines Ordners.
pub fn ordner_zaehler_max(o: &Ordner) -> u64 {
    o.eltern
        .zaehler
        .max(o.name.zaehler)
        .max(o.datenklasse.zaehler)
        .max(o.nur_geraet.zaehler)
        .max(o.zielordner_ki.zaehler)
        .max(o.papierkorb.zaehler)
}

/// Größter Zähler in den Registern einer Datei (samt `wahlen`).
pub fn datei_zaehler_max(d: &Datei) -> u64 {
    d.wahlen.iter().map(|w| w.zaehler).fold(
        d.ordner
            .zaehler
            .max(d.name.zaehler)
            .max(d.klasse_min.zaehler)
            .max(d.papierkorb.zaehler),
        u64::max,
    )
}

/// Trägt kein Register des Ordners einen Zähler über `ZAEHLER_MAX`?
pub fn ordner_zaehler_ok(o: &Ordner) -> bool {
    ordner_zaehler_max(o) <= ZAEHLER_MAX
}

/// Trägt kein Register der Datei (und keine Wahl) einen Zähler über `ZAEHLER_MAX`?
pub fn datei_zaehler_ok(d: &Datei) -> bool {
    datei_zaehler_max(d) <= ZAEHLER_MAX
}

/// Größter bekannter Zähler über alle Register: der nächste Schreibvorgang nimmt diesen Wert + 1.
pub fn groesster_zaehler(
    ordner: &BTreeMap<String, Ordner>,
    dateien: &BTreeMap<String, Datei>,
) -> u64 {
    let o = ordner.values().map(ordner_zaehler_max);
    let d = dateien.values().map(datei_zaehler_max);
    o.chain(d).max().unwrap_or(0)
}

/// Nächster Lamport-Zähler: größter bekannter Wert + 1; `None`, wenn das Ergebnis über `ZAEHLER_MAX`
/// läge (und damit erst recht bei Überlauf von `u64`, nie ein Umlauf auf 0). Lokal wird nie ein Zähler
/// über `ZAEHLER_MAX` vergeben: andere Geräte hielten solche Datensätze an (`datei_zaehler_ok`).
pub fn naechster_zaehler(
    ordner: &BTreeMap<String, Ordner>,
    dateien: &BTreeMap<String, Datei>,
) -> Option<u64> {
    groesster_zaehler(ordner, dateien)
        .checked_add(1)
        .filter(|z| *z <= ZAEHLER_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fas(id: &str, vorgaenger: Option<&str>, zeit: i64) -> Fassung {
        Fassung {
            fas: id.into(),
            vorgaenger: vorgaenger.map(Into::into),
            blob: format!("blob-{id}"),
            size: 10,
            original_sha256: format!("sha-{id}"),
            typ: "pdf".into(),
            geraet: "g_1".into(),
            zeit,
            texte: vec![],
        }
    }

    fn datei(fassungen: Vec<Fassung>, wahlen: Vec<Wahl>) -> Datei {
        Datei {
            id: "dat_1".into(),
            ordner: Register::neu("ord_1".into(), 1, "g_1"),
            name: Register::neu("x.pdf".into(), 1, "g_1"),
            klasse_min: Register::neu("internal".into(), 1, "g_1"),
            nur_geraet_min: false,
            blob_refs: blob_refs_aus(&fassungen),
            fassungen,
            wahlen,
            entfernt: vec![],
            papierkorb: Register::neu(None, 0, "g_1"),
            extra: Default::default(),
        }
    }

    fn wahl(id: &str, gewaehlt: &str, gegen: &[&str], ersetzt: &[&str], z: u64, g: &str) -> Wahl {
        Wahl {
            id: id.into(),
            gewaehlt: gewaehlt.into(),
            gegen: gegen.iter().map(|s| s.to_string()).collect(),
            ersetzt: ersetzt.iter().map(|s| s.to_string()).collect(),
            zaehler: z,
            geraet: g.into(),
        }
    }

    fn permutationen<T: Clone>(v: &[T]) -> Vec<Vec<T>> {
        if v.len() <= 1 {
            return vec![v.to_vec()];
        }
        let mut aus = Vec::new();
        for i in 0..v.len() {
            let mut rest = v.to_vec();
            let x = rest.remove(i);
            for mut p in permutationen(&rest) {
                p.insert(0, x.clone());
                aus.push(p);
            }
        }
        aus
    }

    fn falten(staende: &[Datei]) -> Datei {
        staende
            .iter()
            .skip(1)
            .fold(staende[0].clone(), |a, b| zusammenfuehren_datei(&a, b))
    }

    /// Das Ergebnis darf nicht von der Reihenfolge abhängen.
    fn unabhaengig(staende: &[Datei]) -> Datei {
        let erste = falten(staende);
        for p in permutationen(staende) {
            assert_eq!(falten(&p), erste, "Reihenfolge ändert das Ergebnis");
        }
        // Gruppierung: (a+b)+c gleich a+(b+c)
        if staende.len() == 3 {
            let rechts = zusammenfuehren_datei(
                &staende[0],
                &zusammenfuehren_datei(&staende[1], &staende[2]),
            );
            assert_eq!(rechts, erste);
        }
        erste
    }

    #[test]
    fn register_hoeheres_paar_gewinnt() {
        let a = Register::neu("a".to_string(), 3, "g_1");
        let b = Register::neu("b".to_string(), 3, "g_2");
        let c = Register::neu("c".to_string(), 2, "g_9");
        assert_eq!(gewinner(&a, &b).wert, "b");
        assert_eq!(gewinner(&b, &a).wert, "b");
        assert_eq!(gewinner(&a, &c).wert, "a");
    }

    #[test]
    fn fassungen_nach_fas_vereinigt_ohne_entfernte() {
        let mut a = datei(vec![fas("f1", None, 1), fas("f2", Some("f1"), 2)], vec![]);
        let mut b = datei(vec![fas("f1", None, 1), fas("f3", Some("f1"), 3)], vec![]);
        b.entfernt = vec!["f2".into()];
        a.entfernt = vec![];
        let m = unabhaengig(&[a, b]);
        let ids: Vec<&str> = m.fassungen.iter().map(|f| f.fas.as_str()).collect();
        assert_eq!(ids, ["f1", "f3"]);
        assert_eq!(m.entfernt, ["f2"]);
        assert_eq!(m.blob_refs, ["blob-f1", "blob-f3"]);
    }

    #[test]
    fn gleicher_inhalt_zweier_geraete_bleibt_zwei_fassungen() {
        let mut f2 = fas("f2", None, 1);
        f2.blob = "blob-f1".into();
        let m = zusammenfuehren_datei(
            &datei(vec![fas("f1", None, 1)], vec![]),
            &datei(vec![f2], vec![]),
        );
        assert_eq!(m.fassungen.len(), 2);
        assert_eq!(m.blob_refs, ["blob-f1"]);
        assert!(konflikt(&m));
    }

    #[test]
    fn texte_werden_vereinigt() {
        let t = |rev: &str| Text {
            textrev: rev.into(),
            blob: format!("tb-{rev}"),
            size: 1,
            extraktor: "x".into(),
            extraktor_version: "1".into(),
            marken_version: 1,
            stand: "ok".into(),
            code: None,
            versuche: 0,
            versuche_zeit: 0,
        };
        let mut fa = fas("f1", None, 1);
        fa.texte = vec![t("r1")];
        let mut fb = fas("f1", None, 1);
        fb.texte = vec![t("r2")];
        let m = zusammenfuehren_datei(&datei(vec![fa], vec![]), &datei(vec![fb], vec![]));
        assert_eq!(m.fassungen[0].texte.len(), 2);
        assert!(m.blob_refs.contains(&"tb-r1".to_string()));
        assert!(m.blob_refs.contains(&"tb-r2".to_string()));
    }

    #[test]
    fn zwei_geraete_zwei_koepfe_aktuell_nicht_per_zeit() {
        let a = datei(vec![fas("f1", None, 1), fas("f2", Some("f1"), 100)], vec![]);
        let b = datei(vec![fas("f1", None, 1), fas("f3", Some("f1"), 5)], vec![]);
        let m = unabhaengig(&[a, b]);
        assert_eq!(koepfe(&m), ["f2", "f3"]);
        assert!(konflikt(&m));
        assert_eq!(aktuell(&m), None);
        assert_eq!(gemeinsame_vorgaengerin(&m).as_deref(), Some("f1"));
    }

    #[test]
    fn ohne_gemeinsame_fassung_keine_vorgaengerin() {
        let m = datei(vec![fas("f1", None, 1), fas("f2", None, 2)], vec![]);
        assert!(konflikt(&m));
        assert_eq!(gemeinsame_vorgaengerin(&m), None);
    }

    #[test]
    fn einfache_wahl_dann_weiterbearbeitung_ein_kopf() {
        // A und B gleichzeitig; Wahl B gegen A; danach B -> C.
        let basis = datei(vec![fas("A", None, 1), fas("B", None, 2)], vec![]);
        let w = neue_wahl(&basis, "w1", "B", 5, "g_1");
        assert_eq!(w.gegen, ["A", "B"]);
        let mut m = basis.clone();
        m.wahlen.push(w);
        assert_eq!(koepfe(&m), ["B"]);
        m.fassungen.push(fas("C", Some("B"), 9));
        assert_eq!(koepfe(&m), ["C"]);
        assert_eq!(aktuell(&m).as_deref(), Some("C"));
    }

    #[test]
    fn offline_a_nach_d_ist_neuer_konflikt() {
        let basis = datei(vec![fas("A", None, 1), fas("B", None, 2)], vec![]);
        let mut gewaehlt = basis.clone();
        gewaehlt.wahlen.push(neue_wahl(&basis, "w1", "B", 5, "g_1"));
        gewaehlt.fassungen.push(fas("C", Some("B"), 9));
        let offline = datei(
            vec![fas("A", None, 1), fas("B", None, 2), fas("D", Some("A"), 7)],
            vec![],
        );
        let m = unabhaengig(&[gewaehlt, offline]);
        assert_eq!(koepfe(&m), ["C", "D"]);
        assert!(konflikt(&m));
    }

    #[test]
    fn gegenwahl_ergibt_genau_einen_kopf_in_allen_reihenfolgen() {
        let basis = datei(vec![fas("A", None, 1), fas("B", None, 2)], vec![]);
        let mut g1 = basis.clone();
        g1.wahlen.push(wahl("w1", "A", &["A", "B"], &[], 3, "g_1"));
        let mut g2 = basis.clone();
        g2.wahlen.push(wahl("w2", "B", &["A", "B"], &[], 3, "g_2"));
        let m = unabhaengig(&[g1.clone(), g2.clone(), basis.clone()]);
        assert_eq!(m.wahlen.len(), 2);
        assert_eq!(koepfe(&m), ["B"], "höherer Rang (g_2) gilt");
        // Zähler entscheidet vor Gerät
        let mut g3 = basis.clone();
        g3.wahlen.push(wahl("w3", "A", &["A", "B"], &[], 4, "g_0"));
        let m2 = unabhaengig(&[g3, g2]);
        assert_eq!(koepfe(&m2), ["A"]);
    }

    #[test]
    fn erneute_wahl_nach_gegenwahl_ein_kopf_und_weiterbearbeitung() {
        let basis = datei(vec![fas("A", None, 1), fas("B", None, 2)], vec![]);
        let mut m = basis.clone();
        m.wahlen.push(wahl("w1", "A", &["A", "B"], &[], 3, "g_1"));
        m.wahlen.push(wahl("w2", "B", &["A", "B"], &[], 3, "g_2"));
        assert_eq!(koepfe(&m), ["B"]);
        // Gerät 1 wählt erneut A: ersetzt beide, gegen = Vereinigung
        let neu = neue_wahl(&m, "w3", "A", 4, "g_1");
        assert_eq!(neu.ersetzt, ["w1", "w2"]);
        assert_eq!(neu.gegen, ["A", "B"]);
        m.wahlen.push(neu);
        assert_eq!(koepfe(&m), ["A"]);
        m.fassungen.push(fas("E", Some("A"), 20));
        assert_eq!(koepfe(&m), ["E"], "Weiterbearbeitung konfliktfrei");
    }

    #[test]
    fn w1_a_w2_hoeher_b_w3_b_ersetzt_beide_ein_kopf_b() {
        let basis = datei(vec![fas("A", None, 1), fas("B", None, 2)], vec![]);
        let w1 = wahl("w1", "A", &["A", "B"], &[], 3, "g_1");
        let w2 = wahl("w2", "B", &["A", "B"], &[], 3, "g_2");
        let mut zwischen = basis.clone();
        zwischen.wahlen = vec![w1.clone(), w2.clone()];
        assert_eq!(koepfe(&zwischen), ["B"]);
        let w3 = wahl("w3", "B", &["A", "B"], &["w1", "w2"], 4, "g_3");
        let mut a = basis.clone();
        a.wahlen = vec![w1];
        let mut b = basis.clone();
        b.wahlen = vec![w2];
        let mut c = basis.clone();
        c.wahlen = vec![w3];
        let m = unabhaengig(&[a, b, c]);
        assert_eq!(aktive_wahlen(&m).len(), 1);
        assert_eq!(koepfe(&m), ["B"]);
        assert_eq!(abgeloest(&m).into_iter().collect::<Vec<_>>(), ["A"]);
    }

    #[test]
    fn neue_wahl_uebernimmt_gegen_ersetzter_wahlen() {
        // Wahl B gegen A; später entsteht D auf A (offline). Neue Wahl C... : A bleibt verdrängt.
        let mut d = datei(vec![fas("A", None, 1), fas("B", None, 2)], vec![]);
        d.wahlen.push(wahl("w1", "B", &["A", "B"], &[], 3, "g_1"));
        d.fassungen.push(fas("C", Some("B"), 5));
        let w = neue_wahl(&d, "w2", "C", 6, "g_1");
        assert!(
            w.gegen.contains(&"A".to_string()),
            "verdrängte Fassung bleibt verdrängt"
        );
        assert_eq!(w.ersetzt, ["w1"]);
        d.wahlen.push(w);
        assert_eq!(koepfe(&d), ["C"]);
    }

    // --- Ordner -----------------------------------------------------------

    fn ord(id: &str, eltern: Option<(&str, u64, &str)>) -> Ordner {
        Ordner {
            id: id.into(),
            eltern: match eltern {
                Some((p, z, g)) => Register::neu(Some(p.to_string()), z, g),
                None => Register::neu(None, 0, "g_0"),
            },
            name: Register::neu(id.to_string(), 1, "g_0"),
            datenklasse: Register::neu("erbt".into(), 1, "g_0"),
            nur_geraet: Register::neu(false, 1, "g_0"),
            zielordner_ki: Register::neu(false, 1, "g_0"),
            papierkorb: Register::neu(None, 0, "g_0"),
            extra: Default::default(),
        }
    }

    fn bestand(liste: Vec<Ordner>) -> BTreeMap<String, Ordner> {
        liste.into_iter().map(|o| (o.id.clone(), o)).collect()
    }

    fn falten_ordner(staende: &[BTreeMap<String, Ordner>]) -> BTreeMap<String, Ordner> {
        staende.iter().skip(1).fold(staende[0].clone(), |a, b| {
            zusammenfuehren_ordnerbestand(&a, b)
        })
    }

    #[test]
    fn ordner_a_b_b_a_b_c_in_allen_reihenfolgen() {
        // Gemeinsamer Ausgangsstand: A, B, C an der Wurzel (Zähler 0).
        let basis = || vec![ord("A", None), ord("B", None), ord("C", None)];
        let mit = |aendere: Ordner| {
            let mut b = bestand(basis());
            b.insert(aendere.id.clone(), aendere);
            b
        };
        let g1 = mit(ord("A", Some(("B", 1, "g_1"))));
        let g2 = mit(ord("B", Some(("A", 2, "g_2"))));
        let g3 = mit(ord("B", Some(("C", 3, "g_3"))));
        let alle = [g1.clone(), g2.clone(), g3.clone()];
        let erwartet = falten_ordner(&alle);
        for p in permutationen(&alle) {
            let m = falten_ordner(&p);
            assert_eq!(m, erwartet, "Register hängen von der Reihenfolge ab");
            let e = wirksame_eltern(&m);
            assert_eq!(e["A"], Eltern::Ordner("B".into()));
            assert_eq!(e["B"], Eltern::Ordner("C".into()));
            assert_eq!(e["C"], Eltern::Wurzel);
            assert_eq!(kette(&e, "A"), ["A", "B", "C"]);
        }
        // Register unverändert: A.eltern=B (1), B.eltern=C (3)
        assert_eq!(
            erwartet["A"].eltern,
            Register::neu(Some("B".to_string()), 1, "g_1")
        );
        assert_eq!(
            erwartet["B"].eltern,
            Register::neu(Some("C".to_string()), 3, "g_3")
        );
    }

    #[test]
    fn ordner_zyklus_zunaechst_wiederhergestellt_dann_aufgeloest() {
        let basis = || vec![ord("A", None), ord("B", None), ord("C", None)];
        let mit = |aendere: Ordner| {
            let mut b = bestand(basis());
            b.insert(aendere.id.clone(), aendere);
            b
        };
        let g1 = mit(ord("A", Some(("B", 1, "g_1"))));
        let g2 = mit(ord("B", Some(("A", 2, "g_2"))));
        for (x, y) in [(&g1, &g2), (&g2, &g1)] {
            let m = zusammenfuehren_ordnerbestand(x, y);
            let e = wirksame_eltern(&m);
            assert_eq!(e["B"], Eltern::Ordner("A".into()), "B unter A");
            assert_eq!(
                e["A"],
                Eltern::Wiederhergestellt,
                "A vorläufig wiederhergestellt"
            );
        }
    }

    #[test]
    fn ordner_ohne_bekannten_eltern_ist_wiederhergestellt() {
        let b = bestand(vec![ord("A", Some(("X", 1, "g_1")))]);
        assert_eq!(wirksame_eltern(&b)["A"], Eltern::Wiederhergestellt);
    }

    #[test]
    fn klasse_wird_angehoben_nie_gesenkt_und_unbekannt_sperrt() {
        let mut b = bestand(vec![
            ord("A", None),
            ord("B", Some(("A", 1, "g_1"))),
            ord("C", Some(("B", 1, "g_1"))),
        ]);
        b.get_mut("A").unwrap().datenklasse.wert = "confidential".into();
        b.get_mut("B").unwrap().datenklasse.wert = "public".into();
        let e = wirksame_eltern(&b);
        assert_eq!(wirksame_klasse(&b, &e, "A"), Klasse::Confidential);
        assert_eq!(
            wirksame_klasse(&b, &e, "B"),
            Klasse::Confidential,
            "angehoben"
        );
        assert_eq!(wirksame_klasse(&b, &e, "C"), Klasse::Confidential, "erbt");
        b.get_mut("C").unwrap().datenklasse.wert = "personal".into();
        assert_eq!(wirksame_klasse(&b, &e, "C"), Klasse::Personal);
        b.get_mut("C").unwrap().datenklasse.wert = "geheimer".into();
        assert_eq!(wirksame_klasse(&b, &e, "C"), Klasse::Unbekannt);
    }

    #[test]
    fn nur_geraet_gilt_im_unterbaum() {
        let mut b = bestand(vec![ord("A", None), ord("B", Some(("A", 1, "g_1")))]);
        b.get_mut("A").unwrap().nur_geraet.wert = true;
        let e = wirksame_eltern(&b);
        assert!(wirksam_nur_geraet(&b, &e, "B"));
    }

    #[test]
    fn datei_klasse_ist_maximum() {
        let mut b = bestand(vec![ord("A", None)]);
        b.get_mut("A").unwrap().datenklasse.wert = "internal".into();
        let e = wirksame_eltern(&b);
        let mut d = datei(vec![], vec![]);
        d.ordner.wert = "A".into();
        d.klasse_min.wert = "confidential".into();
        assert_eq!(wirksame_klasse_datei(&b, &e, &d), Klasse::Confidential);
        d.klasse_min.wert = "public".into();
        assert_eq!(wirksame_klasse_datei(&b, &e, &d), Klasse::Internal);
    }
}
