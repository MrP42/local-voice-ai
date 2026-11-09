//! Belegregel E4.2 (ADR-0028): Jeder ausgegebene Aussagesatz braucht einen Beleg; Saetze ohne Beleg
//! werden nicht ausgegeben. Es gibt keine Ausnahme fuer Weltwissen.
//!
//! Belege: Quelle (Satzfenster: Zahlen, Daten, Namen, Polaritaet je zusammenhaengender Teilaussage), Herleitung (das Tor rechnet selbst
//! nach, ganzzahlig in Hundertsteln, keine Gleitkommazahlen), Empfehlung (gekennzeichnet, jede
//! Begruendung ist selbst ein belegter Satz). Rueckfragen und Ansagen sind keine Aussagesaetze.
//! Die Gruende (`Grund`) tragen bewusst keine Inhalte.

use serde::{Deserialize, Serialize};

use std::collections::HashMap;

use crate::einstufung::{Sprache, allgemein_ansage};
use crate::regeltabelle::Regeltabelle;

/// Festbetrag in Hundertsteln (1234,50 = 123450).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Dezimal(pub i64);

impl Dezimal {
    pub fn aus_hundertstel(h: i64) -> Self {
        Self(h)
    }

    /// Ganzzahl in Einheiten.
    pub fn aus_ganzzahl(n: i64) -> Option<Self> {
        n.checked_mul(100).map(Self)
    }
}

/// Kalendertag (proleptisch gregorianisch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Datum {
    pub jahr: i32,
    pub monat: u8,
    pub tag: u8,
}

impl Datum {
    fn gueltig(self) -> bool {
        (1..=12).contains(&self.monat)
            && self.tag >= 1
            && self.tag <= tage_im_monat(self.jahr, self.monat)
    }

    /// Tage seit 1970-01-01 (Hinnant, `days_from_civil`).
    fn tage_seit_epoche(self) -> i64 {
        let j = i64::from(self.jahr) - i64::from(self.monat <= 2);
        let era = j.div_euclid(400);
        let yoe = j.rem_euclid(400);
        let m = i64::from(self.monat);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.tag) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn aus_tagen(tage: i64) -> Option<Self> {
        let z = tage.checked_add(719_468)?;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let tag = u8::try_from(doy - (153 * mp + 2) / 5 + 1).ok()?;
        let monat = u8::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).ok()?;
        let jahr = i32::try_from(yoe + era * 400 + i64::from(monat <= 2)).ok()?;
        Some(Self { jahr, monat, tag })
    }
}

fn tage_im_monat(jahr: i32, monat: u8) -> u8 {
    match monat {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (jahr % 4 == 0 && jahr % 100 != 0) || jahr % 400 == 0 => 29,
        _ => 28,
    }
}

/// Eingaben einer Herleitung mit Herkunft: (quelle_id, woertlicher Auszug).
pub type EingabenBeleg = Vec<(String, String)>;

/// Deterministisch nachgerechnete Herleitung (E4.2): Das Modell rechnet nicht.
/// Operanden von Summe, Produkt und Frist brauchen eine Herkunft (`eingaben_beleg`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Herleitung {
    Summe {
        werte: Vec<Dezimal>,
        ergebnis: Dezimal,
        eingaben_beleg: EingabenBeleg,
    },
    Produkt {
        faktoren: Vec<Dezimal>,
        ergebnis: Dezimal,
        eingaben_beleg: EingabenBeleg,
    },
    /// Minuten seit Mitternacht; `ergebnis` sind die freien Zeitraeume mit mindestens `mindestdauer`.
    /// Jedes belegte Intervall muss samt Start- und Endzeit in einem `eingaben_beleg`-Auszug stehen.
    /// Das Fenster steht ebenfalls in einem Auszug oder ist mit `fenster_von_nutzer` als Eingabe des
    /// Nutzers (Patrick) markiert.
    FreieZeit {
        belegt: Vec<(u32, u32)>,
        fenster: (u32, u32),
        #[serde(default)]
        fenster_von_nutzer: bool,
        mindestdauer: u32,
        ergebnis: Vec<(u32, u32)>,
        eingaben_beleg: EingabenBeleg,
    },
    Frist {
        start_tag: Datum,
        tage: i64,
        ergebnis: Datum,
        eingaben_beleg: EingabenBeleg,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Beleg {
    Quelle { quelle_id: String, auszug: String },
    Herleitung(Herleitung),
    Empfehlung { gruende: Vec<Satz> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SatzArt {
    Aussage,
    Rueckfrage,
    Ansage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Satz {
    pub text: String,
    pub art: SatzArt,
    pub beleg: Option<Beleg>,
}

/// Verwerfungsgrund, ohne Inhalte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Grund {
    OhneBeleg,
    LeereQuelle,
    QuelleUnbekannt,
    AuszugNichtInQuelle,
    ZahlFehltInQuelle,
    ZahlMehrdeutig,
    UnbelegteZahlImSatz,
    DatumFehltInQuelle,
    NameFehltInQuelle,
    PolaritaetGekippt,
    KeineZusammenhaengendeAussage,
    InhaltswortFehlt,
    VerneinungUnzulaessig,
    UngueltigeEingabe,
    EingabenOhneBeleg,
    OperandNichtBelegt,
    RechnungFalsch,
    ErgebnisNichtImSatz,
    TageNichtImSatz,
    EmpfehlungOhneKennzeichnung,
    EmpfehlungOhneGrund,
    BegruendungVerworfen,
    ZuTiefVerschachtelt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pruefergebnis {
    pub ausgegeben: Vec<Satz>,
    pub verworfen: Vec<(Satz, Grund)>,
}

/// Zugriff auf den Volltext einer Quelle (Mail, Kalendereintrag, Datei ...).
pub trait Quellen {
    fn text(&self, quelle_id: &str) -> Option<&str>;
}

/// Einfache Quellenablage fuer Tests und Aufrufer.
#[derive(Debug, Clone, Default)]
pub struct QuellenMap(HashMap<String, String>);

impl QuellenMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mit(mut self, quelle_id: impl Into<String>, text: impl Into<String>) -> Self {
        self.0.insert(quelle_id.into(), text.into());
        self
    }

    pub fn einfuegen(&mut self, quelle_id: impl Into<String>, text: impl Into<String>) {
        self.0.insert(quelle_id.into(), text.into());
    }
}

impl Quellen for QuellenMap {
    fn text(&self, quelle_id: &str) -> Option<&str> {
        self.0.get(quelle_id).map(String::as_str)
    }
}

fn leerraum(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Bekannte Ansagetexte (E4.3): Ansagen der Regeltabelle und die festen Texte. Nur exakt diese
/// Texte gelten als Ansage ohne Beleg; alles andere wird als Aussage geprueft.
#[derive(Debug, Clone, Default)]
pub struct Ansagen {
    vorlagen: Vec<String>,
}

const KEIN_BELEG_DE: (&str, &str) = ("Dazu finde ich keinen Beleg in ", ".");
const KEIN_BELEG_EN: (&str, &str) = ("I can't find any evidence for that in ", ".");

impl Ansagen {
    /// Menge aus den Ansagevorlagen der Tabelle plus festen Texten (allgemein, Rechercheangebot).
    pub fn aus_tabelle(tabelle: &Regeltabelle) -> Self {
        let mut a = Self::default();
        for absicht in &tabelle.absichten {
            a.hinzu(&absicht.ansage_vorlage.de);
            a.hinzu(&absicht.ansage_vorlage.en);
        }
        for sprache in [Sprache::De, Sprache::En] {
            a.hinzu(allgemein_ansage(sprache));
            a.hinzu(keine_quelle_angebot(sprache));
        }
        a
    }

    pub fn hinzu(&mut self, text: &str) {
        self.vorlagen.push(leerraum(text));
    }

    pub fn ist_ansage(&self, text: &str) -> bool {
        let norm = leerraum(text);
        if self.vorlagen.contains(&norm) {
            return true;
        }
        // "Dazu finde ich keinen Beleg in {ort}." mit kurzem, schlichtem Ort.
        [KEIN_BELEG_DE, KEIN_BELEG_EN].iter().any(|(vor, nach)| {
            norm.strip_prefix(vor)
                .and_then(|r| r.strip_suffix(nach))
                .is_some_and(|ort| {
                    !ort.is_empty()
                        && ort.chars().count() <= 60
                        && ort
                            .chars()
                            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '\''))
                })
        })
    }
}

struct Ctx<'a> {
    quellen: &'a dyn Quellen,
    ansagen: &'a Ansagen,
}

const MAX_TIEFE: u8 = 8;

/// Rueckfrage nur als genau ein Fragesatz: endet mit `?`, davor kein weiterer Satzschluss
/// (`.`, `!`, `?`, `;`, `:`) und kein Zeilenumbruch; so wird keine Aussage mitgeschmuggelt.
fn ist_einzelne_frage(text: &str) -> bool {
    let t = text.trim();
    match t.strip_suffix('?') {
        Some(kopf) => {
            !kopf.trim().is_empty() && !kopf.contains(['.', '!', '?', ';', ':', '\n', '\r'])
        }
        None => false,
    }
}

/// Prueft Saetze einzeln; ausgegeben werden nur belegte Aussagen, Rueckfragen (Satz endet mit `?`)
/// und Ansagen mit bekanntem Text. Alles andere wird als Aussage geprueft.
pub fn pruefen(saetze: &[Satz], quellen: &dyn Quellen, ansagen: &Ansagen) -> Pruefergebnis {
    let ctx = Ctx { quellen, ansagen };
    let mut aus = Pruefergebnis {
        ausgegeben: Vec::new(),
        verworfen: Vec::new(),
    };
    for satz in saetze {
        match pruefe_satz(satz, &ctx, 0) {
            Ok(()) => aus.ausgegeben.push(satz.clone()),
            Err(grund) => aus.verworfen.push((satz.clone(), grund)),
        }
    }
    aus
}

fn pruefe_satz(satz: &Satz, ctx: &Ctx, tiefe: u8) -> Result<(), Grund> {
    if tiefe > MAX_TIEFE {
        return Err(Grund::ZuTiefVerschachtelt);
    }
    let nicht_aussage = match satz.art {
        SatzArt::Rueckfrage => ist_einzelne_frage(&satz.text),
        SatzArt::Ansage => ctx.ansagen.ist_ansage(&satz.text),
        SatzArt::Aussage => false,
    };
    if nicht_aussage {
        return Ok(());
    }
    match satz.beleg.as_ref().ok_or(Grund::OhneBeleg)? {
        Beleg::Quelle { quelle_id, auszug } => {
            quelle_binden(ctx, quelle_id, auszug)?;
            pruefe_quelle(&satz.text, auszug)
        }
        Beleg::Herleitung(h) => pruefe_herleitung(&satz.text, h, ctx),
        Beleg::Empfehlung { gruende } => pruefe_empfehlung(&satz.text, gruende, ctx, tiefe),
    }
}

/// Angebot ohne Quelle fuer Weltwissen (E4.2).
pub fn keine_quelle_angebot(sprache: Sprache) -> &'static str {
    match sprache {
        Sprache::De => "Dazu habe ich keine Quelle. Soll ich recherchieren?",
        Sprache::En => "I don't have a source for that. Shall I research it?",
    }
}

/// Kernaussage nicht belegbar (E4.2); `ort` z. B. "deinen Mails".
pub fn kein_beleg(sprache: Sprache, ort: &str) -> String {
    match sprache {
        Sprache::De => format!("{}{ort}{}", KEIN_BELEG_DE.0, KEIN_BELEG_DE.1),
        Sprache::En => format!("{}{ort}{}", KEIN_BELEG_EN.0, KEIN_BELEG_EN.1),
    }
}

/// Feste Aussage fuer ein leeres FreieZeit-Ergebnis.
pub fn keine_freie_zeit(sprache: Sprache) -> &'static str {
    match sprache {
        Sprache::De => "Im Zeitraum gibt es keine freie Zeit.",
        Sprache::En => "There is no free time in that period.",
    }
}

// ---------------------------------------------------------------- Quelle

/// Quelle muss existieren und der Auszug muss woertlich (bis auf Leerraum) darin stehen.
fn quelle_binden(ctx: &Ctx, quelle_id: &str, auszug: &str) -> Result<(), Grund> {
    if quelle_id.trim().is_empty() || auszug.trim().is_empty() {
        return Err(Grund::LeereQuelle);
    }
    let text = ctx.quellen.text(quelle_id).ok_or(Grund::QuelleUnbekannt)?;
    if leerraum(text).contains(&leerraum(auszug)) {
        Ok(())
    } else {
        Err(Grund::AuszugNichtInQuelle)
    }
}

const NEGATIONEN: [&str; 23] = [
    "nicht", "kein", "nie", "niemals", "nichts", "niemand", "ohne", "weder", "nein", "not", "no",
    "never", "cannot", "without", "none", "nobody", "nothing", "neither", "nor", "can't", "won't",
    "don't", "isn't",
];

fn hat_verneinung(text: &str) -> bool {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '\u{2019}'))
        .any(|w| {
            let w = w.replace('\u{2019}', "'");
            NEGATIONEN.contains(&w.as_str()) || w.starts_with("kein") || w.ends_with("n't")
        })
}

/// Haeufige grossgeschriebene Funktions- und Satzanfangswoerter, die keine Namen sind.
const FUNKTIONSWOERTER: &[&str] = &[
    "der",
    "die",
    "das",
    "den",
    "dem",
    "des",
    "ein",
    "eine",
    "einen",
    "einem",
    "einer",
    "ich",
    "du",
    "er",
    "es",
    "sie",
    "wir",
    "ihr",
    "ihnen",
    "ihre",
    "ihren",
    "und",
    "oder",
    "aber",
    "dass",
    "wenn",
    "weil",
    "laut",
    "heute",
    "morgen",
    "gestern",
    "am",
    "im",
    "in",
    "um",
    "bei",
    "mit",
    "von",
    "nach",
    "vor",
    "für",
    "zu",
    "zum",
    "zur",
    "auf",
    "an",
    "aus",
    "als",
    "ab",
    "bis",
    "man",
    "dort",
    "dann",
    "also",
    "ja",
    "hier",
    "da",
    "so",
    "wie",
    "was",
    "wer",
    "wann",
    "wo",
    "warum",
    "welche",
    "welcher",
    "welches",
    "hat",
    "ist",
    "sind",
    "war",
    "wurde",
    "wird",
    "kann",
    "soll",
    "habe",
    "haben",
    "mein",
    "meine",
    "dein",
    "deine",
    "sein",
    "seine",
    "unser",
    "unsere",
    "dieser",
    "diese",
    "dieses",
    "jetzt",
    "bitte",
    "danke",
    "i",
    "a",
    "the",
    "an",
    "and",
    "or",
    "but",
    "it",
    "we",
    "you",
    "they",
    "he",
    "she",
    "there",
    "this",
    "that",
    "these",
    "those",
    "his",
    "her",
    "its",
    "our",
    "your",
    "my",
    "on",
    "at",
    "by",
    "for",
    "from",
    "with",
    "to",
    "of",
    "as",
    "if",
    "then",
    "also",
    "yes",
    "today",
    "tomorrow",
    "yesterday",
    "is",
    "are",
    "was",
    "were",
    "has",
    "have",
    "had",
    "did",
    "does",
    "do",
    "can",
    "will",
    "would",
    "should",
    "could",
    "what",
    "who",
    "when",
    "where",
    "why",
    "which",
    "how",
];

/// Wortstamm fuer den Namensvergleich: haeufige deutsche Endung abgeschnitten (Rest mindestens 4 Zeichen).
fn stamm(wort: &str) -> String {
    for endung in ["en", "es", "s", "e", "n"] {
        if let Some(rest) = wort.strip_suffix(endung)
            && rest.chars().count() >= 4
        {
            return rest.to_owned();
        }
    }
    wort.to_owned()
}

fn woerter_staemme(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(stamm)
        .collect()
}

/// Namen-Heuristik: jedes grossgeschriebene Wort (auch am Satzanfang), das kein Funktionswort ist.
fn namen(text: &str) -> Vec<String> {
    let mut aus = Vec::new();
    for roh in text.split_whitespace() {
        let wort = roh.trim_matches(|c: char| !c.is_alphanumeric());
        let Some(erst) = wort.chars().next() else {
            continue;
        };
        if !erst.is_uppercase() || wort.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        let klein = wort.to_lowercase();
        if FUNKTIONSWOERTER.contains(&klein.as_str()) {
            continue;
        }
        aus.extend(
            klein
                .split(|c: char| !c.is_alphanumeric())
                .filter(|t| !t.is_empty())
                .map(str::to_owned),
        );
    }
    aus
}

/// Zahlen, Daten und Namen des Satzes muessen im Beleg stehen (ohne Polaritaet).
fn inhalt_gedeckt(text: &str, beleg: &str) -> Result<(), Grund> {
    let (satz_daten, satz_zahlen) = daten_und_zahlen(text);
    let (beleg_daten, beleg_zahlen) = daten_und_zahlen(beleg);
    if satz_daten.iter().any(|d| !beleg_daten.contains(d)) {
        return Err(Grund::DatumFehltInQuelle);
    }
    let werte = eindeutige_werte(&beleg_zahlen);
    for z in &satz_zahlen {
        match z {
            Zahl::Mehrdeutig => return Err(Grund::ZahlMehrdeutig),
            Zahl::Wert(w) if !werte.contains(w) => return Err(Grund::ZahlFehltInQuelle),
            Zahl::Wert(_) => {}
        }
    }
    let staemme = woerter_staemme(beleg);
    if namen(text).iter().any(|n| !staemme.contains(&stamm(n))) {
        return Err(Grund::NameFehltInQuelle);
    }
    Ok(())
}

fn pruefe_quelle(text: &str, auszug: &str) -> Result<(), Grund> {
    let zusammenhaengend = aussage_gebunden(text, auszug);
    if !zusammenhaengend && hat_verneinung(text) != hat_verneinung(auszug) {
        return Err(Grund::PolaritaetGekippt);
    }
    inhalt_gedeckt(text, auszug)?;
    inhaltswoerter_gedeckt(text, auszug)?;
    if zusammenhaengend {
        Ok(())
    } else {
        Err(Grund::KeineZusammenhaengendeAussage)
    }
}

/// Nebenordnende Konjunktionen (DE und EN), an denen eine Aussage in Teilaussagen zerfaellt.
const KONJUNKTIONEN: &[&str] = &[
    "und",
    "aber",
    "oder",
    "sondern",
    "doch",
    "jedoch",
    "während",
    "weil",
    "da",
    "obwohl",
    "wohingegen",
    "and",
    "but",
    "or",
    "while",
    "whereas",
    "because",
    "although",
];

const MONATE: &[&str] = &[
    "januar",
    "februar",
    "märz",
    "maerz",
    "april",
    "mai",
    "juni",
    "juli",
    "august",
    "september",
    "oktober",
    "november",
    "dezember",
];

/// Hilfs- und Modalverben sowie Verneinungen am Anfang: die Teilaussage hat kein eigenes Subjekt.
const OHNE_SUBJEKT_ANFANG: &[&str] = &[
    "hat", "hatte", "hatten", "haben", "ist", "war", "waren", "sind", "wird", "wurde", "wurden",
    "werden", "kann", "konnte", "muss", "soll", "sollte", "darf", "will", "wollte", "has", "have",
    "had", "is", "was", "were", "are", "did", "does", "do", "will", "would", "can", "could",
    "should", "must", "may", "might", "shall",
];

fn wort_klein(roh: &str) -> String {
    roh.trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// Satzgrenze: `.` (nur vor Leerraum oder Ende, nicht bei Dezimal-, Datums- oder Monatspunkt),
/// `!`, `?`, `;` und Zeilenumbruch.
fn saetze_teilen(text: &str) -> Vec<String> {
    let zeichen: Vec<char> = text.chars().collect();
    let mut aus = Vec::new();
    let mut aktuell = String::new();
    for (i, &c) in zeichen.iter().enumerate() {
        let grenze = match c {
            '!' | '?' | ';' | '\n' | '\r' => true,
            '.' => {
                let danach = zeichen.get(i + 1);
                let ende_oder_leer = danach.is_none_or(|d| d.is_whitespace());
                let nach_ziffer = i > 0 && zeichen[i - 1].is_ascii_digit();
                let monat = nach_ziffer
                    && zeichen[i + 1..]
                        .iter()
                        .collect::<String>()
                        .split_whitespace()
                        .next()
                        .is_some_and(|w| MONATE.contains(&wort_klein(w).as_str()));
                ende_oder_leer && !monat
            }
            _ => false,
        };
        if grenze {
            aus.push(std::mem::take(&mut aktuell));
        } else {
            aktuell.push(c);
        }
    }
    aus.push(aktuell);
    aus
}

/// Teilaussagen: Saetze, innerhalb davon getrennt an nebenordnenden Konjunktionen. Leere Reste entfallen.
fn teilaussagen(text: &str) -> Vec<String> {
    let mut aus = Vec::new();
    for satz in saetze_teilen(text) {
        let mut klausel: Vec<&str> = Vec::new();
        for wort in satz.split_whitespace() {
            if KONJUNKTIONEN.contains(&wort_klein(wort).as_str()) {
                if !klausel.is_empty() {
                    aus.push(klausel.join(" "));
                    klausel.clear();
                }
            } else {
                klausel.push(wort);
            }
        }
        if !klausel.is_empty() {
            aus.push(klausel.join(" "));
        }
    }
    aus.into_iter()
        .filter(|k| k.chars().any(|c| c.is_alphanumeric()))
        .collect()
}

fn verneinungen_zaehlen(text: &str) -> usize {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '\u{2019}'))
        .filter(|w| {
            let w = w.replace('\u{2019}', "'");
            NEGATIONEN.contains(&w.as_str()) || w.starts_with("kein") || w.ends_with("n't")
        })
        .count()
}

fn ohne_eigenes_subjekt(klausel: &str) -> bool {
    klausel.split_whitespace().next().is_some_and(|w| {
        let w = wort_klein(w);
        OHNE_SUBJEKT_ANFANG.contains(&w.as_str()) || NEGATIONEN.contains(&w.as_str())
    })
}

/// Jede Teilaussage des Satzes braucht eine zusammenhaengende Quell-Teilaussage mit allen Inhaltswoertern,
/// Zahlen, Daten und Namen und gleicher Polaritaet. Subjektlose Teilaussagen und Mehrfachverneinungen
/// werden nicht freigegeben.
fn aussage_gebunden(text: &str, auszug: &str) -> bool {
    let ausgabe = teilaussagen(text);
    if ausgabe.is_empty() {
        return false;
    }
    let quelle: Vec<(String, usize)> = teilaussagen(auszug)
        .into_iter()
        .map(|k| {
            let n = verneinungen_zaehlen(&k);
            (k, n)
        })
        .collect();
    ausgabe.iter().all(|o| {
        let neg = verneinungen_zaehlen(o);
        neg <= 1
            && !ohne_eigenes_subjekt(o)
            && quelle.iter().any(|(q, n)| {
                *n == neg && inhalt_gedeckt(o, q).is_ok() && inhaltswoerter_gedeckt(o, q).is_ok()
            })
    })
}

/// Weitere Funktionswoerter (Hilfsverben, Gradpartikeln, Pronomen, Praepositionen), die als
/// Inhaltswort nichts behaupten. Verneinungen (`NEGATIONEN`) zaehlen ebenfalls nicht als Inhaltswort,
/// sie sind ueber die Polaritaet geprueft.
const FUNKTIONSWOERTER_PLUS: &[&str] = &[
    "wurden", "werden", "worden", "waren", "wäre", "wären", "sei", "seid", "seien", "bin", "bist",
    "hatte", "hatten", "hast", "habt", "wirst", "werdet", "kannst", "können", "konnte", "konnten",
    "muss", "müssen", "musste", "darf", "dürfen", "wollen", "sollen", "sollte", "auch", "noch",
    "nur", "schon", "sehr", "uns", "mir", "mich", "dir", "dich", "ihm", "ihn", "euch", "sich",
    "seinen", "seiner", "seinem", "seines", "ihrer", "ihrem", "ihres", "unseren", "unserer",
    "diesen", "diesem", "jener", "denn", "doch", "mal", "etwa", "über", "unter", "durch", "gegen",
    "seit", "während", "zwischen", "been", "being", "shall", "may", "might", "must", "very",
    "just", "only", "too", "than", "into", "about", "over", "under", "their", "them", "him",
    "keine", "keinen", "keiner",
];

fn ist_funktionswort(klein: &str) -> bool {
    FUNKTIONSWOERTER.contains(&klein)
        || FUNKTIONSWOERTER_PLUS.contains(&klein)
        || NEGATIONEN.contains(&klein)
}

/// Endungen, laengste zuerst (DE -e, -en, -er, -es, -em, -n, -s, -t, -et, -st; EN -s, -es, -ed, -ing).
const FLEXION: [&str; 12] = [
    "ing", "en", "er", "es", "em", "et", "st", "ed", "e", "n", "s", "t",
];

/// Einfacher Stamm: hoechstens eine gaengige Flexionsendung abgeschnitten, Rest mindestens 3 Zeichen.
fn flexion_ab(wort: &str) -> String {
    FLEXION
        .iter()
        .find_map(|e| wort.strip_suffix(e).filter(|r| r.chars().count() >= 3))
        .unwrap_or(wort)
        .to_owned()
}

/// Vergleichsformen eines Wortes: Wort selbst und Stamm (so trifft "Angebot" auf "Angebote");
/// bei Partizipien (ge-...-t/-en) zusaetzlich ohne ge-Praefix.
fn stammformen(wort: &str) -> Vec<String> {
    let mut formen = vec![wort.to_owned(), flexion_ab(wort)];
    if let Some(ohne) = wort.strip_prefix("ge")
        && ohne.chars().count() >= 3
        && (wort.ends_with('t') || wort.ends_with("en"))
    {
        formen.push(ohne.to_owned());
        formen.push(flexion_ab(ohne));
    }
    formen
}

/// Inhaltswoerter: Woerter ohne Ziffern, mindestens 3 Buchstaben, kein Funktionswort.
fn inhaltswoerter(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| {
            w.chars().filter(|c| c.is_alphabetic()).count() >= 3
                && !w.chars().any(|c| c.is_ascii_digit())
                && !ist_funktionswort(w)
        })
        .map(str::to_owned)
        .collect()
}

/// Jedes Inhaltswort des Satzes muss (ueber den einfachen Stamm, mit Wortgrenze) im Beleg stehen.
fn inhaltswoerter_gedeckt(text: &str, beleg: &str) -> Result<(), Grund> {
    let vorrat: Vec<String> = inhaltswoerter_alle(beleg)
        .iter()
        .flat_map(|w| stammformen(w))
        .collect();
    let fehlt = inhaltswoerter(text)
        .iter()
        .any(|w| !stammformen(w).iter().any(|f| vorrat.contains(f)));
    if fehlt {
        Err(Grund::InhaltswortFehlt)
    } else {
        Ok(())
    }
}

/// Alle Woerter des Belegs (auch Funktionswoerter), damit z. B. "wurde" im Auszug zaehlt.
fn inhaltswoerter_alle(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

// ---------------------------------------------------------------- Zahlen und Daten

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zahl {
    /// Normierter Wert in Millionstel.
    Wert(i128),
    /// Schreibweise nicht eindeutig (z. B. "1.234") oder nicht darstellbar.
    Mehrdeutig,
}

fn eindeutige_werte(zahlen: &[Zahl]) -> Vec<i128> {
    zahlen
        .iter()
        .filter_map(|z| match z {
            Zahl::Wert(w) => Some(*w),
            Zahl::Mehrdeutig => None,
        })
        .collect()
}

const MIKRO: i128 = 1_000_000;

fn ganz(s: &str) -> Zahl {
    s.parse::<i128>()
        .ok()
        .and_then(|n| n.checked_mul(MIKRO))
        .map_or(Zahl::Mehrdeutig, Zahl::Wert)
}

fn dezimal(ganzteil: &str, bruch: &str) -> Zahl {
    if bruch.len() > 6 {
        return Zahl::Mehrdeutig;
    }
    let (Zahl::Wert(g), Ok(b)) = (ganz(ganzteil), format!("{bruch:0<6}").parse::<i128>()) else {
        return Zahl::Mehrdeutig;
    };
    g.checked_add(b).map_or(Zahl::Mehrdeutig, Zahl::Wert)
}

/// Gueltige Tausendergruppen: erste Gruppe 1-3 Ziffern, alle weiteren genau 3.
fn gruppen_ok(teile: &[&str]) -> bool {
    teile.len() >= 2 && (1..=3).contains(&teile[0].len()) && teile[1..].iter().all(|g| g.len() == 3)
}

fn zerlegen(token: &str) -> Vec<Zahl> {
    token.split(['.', ',']).map(ganz).collect()
}

/// Strenger Zahlenparser. Ein einzelner Trenner mit genau drei Ziffern danach ("1.234") ist
/// mehrdeutig. Ungueltige Gruppierungen ("1,2,3") sind keine Zahl, sondern getrennte Zahlen.
fn parse_token(token: &str) -> Vec<Zahl> {
    let trenner: Vec<char> = token.chars().filter(|c| matches!(c, '.' | ',')).collect();
    let Some(&letzter) = trenner.last() else {
        return vec![ganz(token)];
    };
    if trenner.iter().all(|c| *c == letzter) {
        let teile: Vec<&str> = token.split(letzter).collect();
        if trenner.len() == 1 {
            let (vor, nach) = (teile[0], teile[1]);
            if nach.len() == 3 && (1..=3).contains(&vor.len()) && vor != "0" {
                return vec![Zahl::Mehrdeutig];
            }
            return vec![dezimal(vor, nach)];
        }
        if gruppen_ok(&teile) {
            return vec![ganz(&teile.concat())];
        }
        return zerlegen(token);
    }
    // Gemischt: der letzte Trenner ist der Dezimaltrenner, davor nur Tausendergruppen.
    let Some((links, rechts)) = token.rsplit_once(letzter) else {
        return zerlegen(token);
    };
    let tausender = if letzter == ',' { '.' } else { ',' };
    let teile: Vec<&str> = links.split(tausender).collect();
    if !links.contains(letzter) && gruppen_ok(&teile) && !rechts.is_empty() {
        return vec![dezimal(&teile.concat(), rechts)];
    }
    zerlegen(token)
}

fn ist_minus(c: char) -> bool {
    matches!(c, '-' | '\u{2212}')
}

#[cfg(test)]
fn zahlen_von(text: &str) -> Vec<Zahl> {
    daten_und_zahlen(text).1
}

// ---------------------------------------------------------------- Daten

/// Findet Daten (`TT.MM.JJJJ`, `T.M.JJ`, `JJJJ-MM-TT`) und gibt ihre Zeichenbereiche zurueck.
fn finde_daten(zeichen: &[char]) -> Vec<(usize, usize, Datum)> {
    let mut aus = Vec::new();
    let mut i = 0;
    while i < zeichen.len() {
        let frei = i == 0 || !zeichen[i - 1].is_ascii_digit();
        if !(zeichen[i].is_ascii_digit() && frei) {
            i += 1;
            continue;
        }
        match datum_ab(zeichen, i) {
            Some((ende, d)) if d.gueltig() => {
                aus.push((i, ende, d));
                i = ende;
            }
            _ => i += 1,
        }
    }
    aus
}

fn ziffern(z: &[char], mut i: usize, min: usize, max: usize) -> Option<(u32, usize)> {
    let start = i;
    let mut w: u32 = 0;
    while i < z.len() && i - start < max && z[i].is_ascii_digit() {
        w = w * 10 + z[i].to_digit(10)?;
        i += 1;
    }
    (i - start >= min).then_some((w, i))
}

fn datum_ab(z: &[char], i: usize) -> Option<(usize, Datum)> {
    let nach_ziffer = |ende: usize| ende < z.len() && z[ende].is_ascii_digit();
    if let Some((jahr, p)) = ziffern(z, i, 4, 4)
        && p < z.len()
        && z[p] == '-'
        && let Some((monat, p)) = ziffern(z, p + 1, 1, 2)
        && p < z.len()
        && z[p] == '-'
        && let Some((tag, p)) = ziffern(z, p + 1, 1, 2)
        && !nach_ziffer(p)
    {
        let d = Datum {
            jahr: i32::try_from(jahr).ok()?,
            monat: u8::try_from(monat).ok()?,
            tag: u8::try_from(tag).ok()?,
        };
        return Some((p, d));
    }
    let (tag, p) = ziffern(z, i, 1, 2)?;
    if p >= z.len() || z[p] != '.' {
        return None;
    }
    let (monat, p) = ziffern(z, p + 1, 1, 2)?;
    if p >= z.len() || z[p] != '.' {
        return None;
    }
    let (jahr, p) = ziffern(z, p + 1, 2, 4)?;
    if nach_ziffer(p) {
        return None;
    }
    let jahr = if jahr < 100 { jahr + 2000 } else { jahr };
    let d = Datum {
        jahr: i32::try_from(jahr).ok()?,
        monat: u8::try_from(monat).ok()?,
        tag: u8::try_from(tag).ok()?,
    };
    Some((p, d))
}

/// Daten und Zahlen eines Textes; Daten werden vor der Zahlensuche ausgeblendet, ein fuehrendes
/// `-` oder `−` unmittelbar vor der Ziffer (nicht nach Buchstabe/Ziffer) ist ein Vorzeichen.
fn daten_und_zahlen(text: &str) -> (Vec<Datum>, Vec<Zahl>) {
    let mut zeichen: Vec<char> = text.chars().collect();
    let daten = finde_daten(&zeichen);
    for (von, bis, _) in &daten {
        zeichen[*von..*bis].fill(' ');
    }
    let mut zahlen = Vec::new();
    let mut i = 0;
    while i < zeichen.len() {
        let frei = i == 0 || !zeichen[i - 1].is_ascii_digit();
        if !(zeichen[i].is_ascii_digit() && frei) {
            i += 1;
            continue;
        }
        let negativ =
            i >= 1 && ist_minus(zeichen[i - 1]) && (i == 1 || !zeichen[i - 2].is_alphanumeric());
        let mut j = i;
        loop {
            while j < zeichen.len() && zeichen[j].is_ascii_digit() {
                j += 1;
            }
            let weiter = j + 1 < zeichen.len()
                && matches!(zeichen[j], '.' | ',')
                && zeichen[j + 1].is_ascii_digit();
            if !weiter {
                break;
            }
            j += 1;
        }
        let token: String = zeichen[i..j].iter().collect();
        let mut teile = parse_token(&token);
        if negativ && let Some(Zahl::Wert(w)) = teile.first_mut() {
            *w = -*w;
        }
        zahlen.extend(teile);
        i = j;
    }
    (daten.into_iter().map(|(_, _, d)| d).collect(), zahlen)
}

fn dez_mikro(d: Dezimal) -> i128 {
    i128::from(d.0) * 10_000
}

fn zahl_im_text(text: &str, hundertstel: i64) -> bool {
    eindeutige_werte(&daten_und_zahlen(text).1).contains(&(i128::from(hundertstel) * 10_000))
}

/// Jede Zahl im Satz muss erlaubt sein (Operand oder Ergebnis); mehrdeutige Schreibweise verwirft.
fn nur_erlaubte_zahlen(text: &str, erlaubt: &[i128]) -> Result<(), Grund> {
    for z in daten_und_zahlen(text).1 {
        match z {
            Zahl::Mehrdeutig => return Err(Grund::ZahlMehrdeutig),
            Zahl::Wert(w) if !erlaubt.contains(&w) => return Err(Grund::UnbelegteZahlImSatz),
            Zahl::Wert(_) => {}
        }
    }
    Ok(())
}

/// Uhrzeiten eines Textes in Minuten: `H:MM`, `H:MM AM/PM`, `H Uhr`, `H AM/PM`.
fn zeiten_im_text(text: &str) -> Vec<u32> {
    let z: Vec<char> = text.to_lowercase().chars().collect();
    let wort_ab = |mut p: usize, w: &str| -> Option<usize> {
        while p < z.len() && z[p] == ' ' {
            p += 1;
        }
        let ende = p + w.chars().count();
        let passt = z
            .get(p..ende)
            .is_some_and(|s| s.iter().copied().eq(w.chars()));
        (passt && z.get(ende).is_none_or(|c| !c.is_alphanumeric())).then_some(ende)
    };
    let mut aus = Vec::new();
    let mut i = 0;
    while i < z.len() {
        let davor_zahl =
            i > 0 && (z[i - 1].is_ascii_digit() || matches!(z[i - 1], ':' | '.' | ','));
        if !z[i].is_ascii_digit() || davor_zahl {
            i += 1;
            continue;
        }
        let Some((h, mut ende)) = ziffern(&z, i, 1, 2) else {
            i += 1;
            continue;
        };
        let mut minute = 0;
        let mut mit_minute = false;
        if ende < z.len()
            && z[ende] == ':'
            && let Some((m, q)) = ziffern(&z, ende + 1, 2, 2)
            && m < 60
            && !z.get(q).is_some_and(char::is_ascii_digit)
        {
            minute = m;
            ende = q;
            mit_minute = true;
        }
        let am = wort_ab(ende, "am");
        let pm = wort_ab(ende, "pm");
        let uhr = wort_ab(ende, "uhr");
        let wert = match (am, pm, uhr) {
            (Some(e), _, _) if (1..=12).contains(&h) => Some((h % 12, e)),
            (_, Some(e), _) if (1..=12).contains(&h) => Some((h % 12 + 12, e)),
            (_, _, Some(e)) if h <= 24 => Some((h, e)),
            _ if mit_minute && h <= 24 => Some((h, ende)),
            _ => None,
        };
        match wert {
            Some((stunde, e)) => {
                aus.push(stunde * 60 + minute);
                i = e.max(i + 1);
            }
            None => i = ende.max(i + 1),
        }
    }
    aus
}

// ---------------------------------------------------------------- Herleitung

fn eingaben_pruefen(ctx: &Ctx, eingaben: &EingabenBeleg) -> Result<Vec<String>, Grund> {
    if eingaben.is_empty() {
        return Err(Grund::EingabenOhneBeleg);
    }
    for (id, auszug) in eingaben {
        quelle_binden(ctx, id, auszug)?;
    }
    Ok(eingaben.iter().map(|(_, a)| a.clone()).collect())
}

fn operanden_belegt(auszuege: &[String], operanden: &[i128]) -> Result<(), Grund> {
    let werte: Vec<i128> = auszuege
        .iter()
        .flat_map(|a| eindeutige_werte(&daten_und_zahlen(a).1))
        .collect();
    if operanden.iter().all(|o| werte.contains(o)) {
        Ok(())
    } else {
        Err(Grund::OperandNichtBelegt)
    }
}

/// Start und Ende eines Intervalls stehen zusammen in einem Auszug.
fn intervall_belegt(auszuege: &[Vec<u32>], (von, bis): (u32, u32)) -> bool {
    auszuege
        .iter()
        .any(|zeiten| zeiten.contains(&von) && zeiten.contains(&bis))
}

fn freie_zeit_belegt(
    auszuege: &[String],
    belegt: &[(u32, u32)],
    fenster: (u32, u32),
    fenster_von_nutzer: bool,
) -> Result<(), Grund> {
    let zeiten: Vec<Vec<u32>> = auszuege.iter().map(|a| zeiten_im_text(a)).collect();
    let alle_belegt = belegt.iter().all(|i| intervall_belegt(&zeiten, *i));
    if alle_belegt && (fenster_von_nutzer || intervall_belegt(&zeiten, fenster)) {
        Ok(())
    } else {
        Err(Grund::OperandNichtBelegt)
    }
}

/// Rahmenwoerter je Herleitungsart: Satzrahmen, den das Modell frei formulieren darf. Alle anderen
/// Inhaltswoerter eines Herleitungssatzes muessen in einem `eingaben_beleg`-Auszug stehen.
const RAHMEN_SUMME: &[&str] = &[
    "summe",
    "insgesamt",
    "zusammen",
    "beträgt",
    "ergibt",
    "macht",
    "total",
    "sum",
    "amount",
    "comes",
    "euro",
    "eur",
    "stunden",
    "hours",
    "tage",
    "days",
];
/// Produkt: Rahmen der Summe plus Multiplikationswoerter.
const RAHMEN_PRODUKT_PLUS: &[&str] = &["mal", "je", "pro", "per", "times", "satz", "rate"];
const RAHMEN_FREIE_ZEIT: &[&str] = &[
    "frei",
    "freie",
    "zeit",
    "termin",
    "termine",
    "zwischen",
    "uhr",
    "bist",
    "hast",
    "available",
    "free",
    "between",
    "slot",
    "slots",
];
const RAHMEN_FRIST: &[&str] = &[
    "frist", "endet", "läuft", "ab", "bis", "tage", "tagen", "deadline", "ends", "days", "due",
];
/// Wochentage und Monatsnamen (DE und EN) sind in jedem Herleitungssatz als Rahmen erlaubt.
const RAHMEN_KALENDER: &[&str] = &[
    "montag",
    "dienstag",
    "mittwoch",
    "donnerstag",
    "freitag",
    "samstag",
    "sonntag",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "januar",
    "februar",
    "märz",
    "april",
    "mai",
    "juni",
    "juli",
    "august",
    "september",
    "oktober",
    "november",
    "dezember",
    "january",
    "february",
    "march",
    "june",
    "july",
    "october",
    "december",
];

fn rahmen_von(h: &Herleitung) -> (Vec<&'static [&'static str]>, &EingabenBeleg) {
    match h {
        Herleitung::Summe { eingaben_beleg, .. } => (vec![RAHMEN_SUMME], eingaben_beleg),
        Herleitung::Produkt { eingaben_beleg, .. } => {
            (vec![RAHMEN_SUMME, RAHMEN_PRODUKT_PLUS], eingaben_beleg)
        }
        Herleitung::FreieZeit { eingaben_beleg, .. } => (vec![RAHMEN_FREIE_ZEIT], eingaben_beleg),
        Herleitung::Frist { eingaben_beleg, .. } => (vec![RAHMEN_FRIST], eingaben_beleg),
    }
}

/// Inhaltswoerter eines Herleitungssatzes: Rahmenwort der Art oder Wort eines Eingabe-Auszugs.
fn herleitung_woerter(text: &str, h: &Herleitung) -> Result<(), Grund> {
    let (rahmen, eingaben) = rahmen_von(h);
    let mut vorrat: Vec<String> = rahmen
        .iter()
        .chain([&RAHMEN_KALENDER])
        .flat_map(|liste| liste.iter())
        .flat_map(|w| stammformen(w))
        .collect();
    for (_, auszug) in eingaben {
        vorrat.extend(
            inhaltswoerter_alle(auszug)
                .iter()
                .flat_map(|w| stammformen(w)),
        );
    }
    let fehlt = inhaltswoerter(text)
        .iter()
        .any(|w| !stammformen(w).iter().any(|f| vorrat.contains(f)));
    if fehlt {
        Err(Grund::InhaltswortFehlt)
    } else {
        Ok(())
    }
}

fn pruefe_herleitung(text: &str, h: &Herleitung, ctx: &Ctx) -> Result<(), Grund> {
    pruefe_herleitung_rechnung(text, h, ctx)?;
    let vorlage = [Sprache::De, Sprache::En]
        .iter()
        .any(|s| leerraum(text) == keine_freie_zeit(*s));
    if vorlage {
        return Ok(());
    }
    herleitung_woerter(text, h)
}

fn pruefe_herleitung_rechnung(text: &str, h: &Herleitung, ctx: &Ctx) -> Result<(), Grund> {
    match h {
        Herleitung::Summe {
            werte,
            ergebnis,
            eingaben_beleg,
        } => {
            let auszuege = eingaben_pruefen(ctx, eingaben_beleg)?;
            if werte.is_empty() {
                return Err(Grund::UngueltigeEingabe);
            }
            let operanden: Vec<i128> = werte.iter().map(|w| dez_mikro(*w)).collect();
            operanden_belegt(&auszuege, &operanden)?;
            let summe = werte
                .iter()
                .try_fold(0i64, |acc, w| acc.checked_add(w.0))
                .ok_or(Grund::UngueltigeEingabe)?;
            zahl_pruefen(text, summe, *ergebnis, operanden)
        }
        Herleitung::Produkt {
            faktoren,
            ergebnis,
            eingaben_beleg,
        } => {
            let auszuege = eingaben_pruefen(ctx, eingaben_beleg)?;
            let operanden: Vec<i128> = faktoren.iter().map(|w| dez_mikro(*w)).collect();
            operanden_belegt(&auszuege, &operanden)?;
            let produkt = produkt_hundertstel(faktoren).ok_or(Grund::UngueltigeEingabe)?;
            zahl_pruefen(text, produkt, *ergebnis, operanden)
        }
        Herleitung::FreieZeit {
            belegt,
            fenster,
            fenster_von_nutzer,
            mindestdauer,
            ergebnis,
            eingaben_beleg,
        } => {
            let auszuege = eingaben_pruefen(ctx, eingaben_beleg)?;
            freie_zeit_belegt(&auszuege, belegt, *fenster, *fenster_von_nutzer)?;
            let frei =
                freie_zeit(belegt, *fenster, *mindestdauer).ok_or(Grund::UngueltigeEingabe)?;
            let mut gegeben = ergebnis.clone();
            gegeben.sort_unstable();
            if frei != gegeben {
                return Err(Grund::RechnungFalsch);
            }
            if frei.is_empty() {
                let norm = leerraum(text);
                let vorlage = [Sprache::De, Sprache::En]
                    .iter()
                    .any(|s| norm == keine_freie_zeit(*s));
                return if vorlage {
                    Ok(())
                } else {
                    Err(Grund::ErgebnisNichtImSatz)
                };
            }
            if hat_verneinung(text) {
                return Err(Grund::VerneinungUnzulaessig);
            }
            let grenzen: Vec<u32> = frei.iter().flat_map(|(s, e)| [*s, *e]).collect();
            let im_text = zeiten_im_text(text);
            if im_text.iter().any(|t| !grenzen.contains(t)) {
                return Err(Grund::UnbelegteZahlImSatz);
            }
            if grenzen.iter().all(|g| im_text.contains(g)) {
                Ok(())
            } else {
                Err(Grund::ErgebnisNichtImSatz)
            }
        }
        Herleitung::Frist {
            start_tag,
            tage,
            ergebnis,
            eingaben_beleg,
        } => {
            let auszuege = eingaben_pruefen(ctx, eingaben_beleg)?;
            if !start_tag.gueltig() || !ergebnis.gueltig() {
                return Err(Grund::UngueltigeEingabe);
            }
            let tage_mikro = i128::from(*tage) * MIKRO;
            let daten: Vec<Datum> = auszuege
                .iter()
                .flat_map(|a| daten_und_zahlen(a).0)
                .collect();
            if !daten.contains(start_tag) {
                return Err(Grund::OperandNichtBelegt);
            }
            operanden_belegt(&auszuege, &[tage_mikro])?;
            let ziel = start_tag
                .tage_seit_epoche()
                .checked_add(*tage)
                .ok_or(Grund::UngueltigeEingabe)?;
            if Datum::aus_tagen(ziel) != Some(*ergebnis) {
                return Err(Grund::RechnungFalsch);
            }
            if !datum_im_text(text, *ergebnis) {
                return Err(Grund::ErgebnisNichtImSatz);
            }
            if eindeutige_werte(&daten_und_zahlen(text).1).contains(&tage_mikro) {
                Ok(())
            } else {
                Err(Grund::TageNichtImSatz)
            }
        }
    }
}

fn zahl_pruefen(
    text: &str,
    berechnet: i64,
    angegeben: Dezimal,
    mut erlaubt: Vec<i128>,
) -> Result<(), Grund> {
    if berechnet != angegeben.0 {
        return Err(Grund::RechnungFalsch);
    }
    erlaubt.push(dez_mikro(angegeben));
    nur_erlaubte_zahlen(text, &erlaubt)?;
    if zahl_im_text(text, berechnet) {
        Ok(())
    } else {
        Err(Grund::ErgebnisNichtImSatz)
    }
}

/// Wortgrenzen-Suche (Kleinschreibung erwartet): davor/danach weder Buchstabe/Ziffer noch Doppelpunkt davor.
fn enthaelt_grenze(heu: &str, nadel: &str) -> bool {
    heu.match_indices(nadel).any(|(p, _)| {
        let davor = heu[..p].chars().next_back();
        let danach = heu[p + nadel.len()..].chars().next();
        davor.is_none_or(|c| !c.is_alphanumeric() && c != ':')
            && danach.is_none_or(|c| !c.is_alphanumeric())
    })
}

fn datum_im_text(text: &str, d: Datum) -> bool {
    let zeichen: Vec<char> = text.chars().collect();
    if finde_daten(&zeichen).iter().any(|(_, _, x)| *x == d) {
        return true;
    }
    const DE: [&str; 12] = [
        "januar",
        "februar",
        "märz",
        "april",
        "mai",
        "juni",
        "juli",
        "august",
        "september",
        "oktober",
        "november",
        "dezember",
    ];
    const EN: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let klein = text.to_lowercase();
    let i = usize::from(d.monat) - 1;
    [
        format!("{}. {} {}", d.tag, DE[i], d.jahr),
        format!("{} {}, {}", EN[i], d.tag, d.jahr),
        format!("{} {} {}", d.tag, EN[i], d.jahr),
    ]
    .iter()
    .any(|n| enthaelt_grenze(&klein, n))
}

/// Produkt in Hundertsteln, kaufmaennisch gerundet (halb von Null weg).
fn produkt_hundertstel(faktoren: &[Dezimal]) -> Option<i64> {
    let (erster, rest) = faktoren.split_first()?;
    let mut p = i128::from(erster.0);
    for f in rest {
        p = p.checked_mul(i128::from(f.0))?;
        // Nach jedem Faktor auf Hundertstel zurueck, damit nichts ueberlaeuft.
        let (q, r) = (p / 100, p % 100);
        p = if r.abs() >= 50 { q + p.signum() } else { q };
    }
    i64::try_from(p).ok()
}

fn freie_zeit(
    belegt: &[(u32, u32)],
    fenster: (u32, u32),
    mindestdauer: u32,
) -> Option<Vec<(u32, u32)>> {
    const TAG: u32 = 24 * 60;
    if fenster.0 >= fenster.1 || fenster.1 > TAG || belegt.iter().any(|(s, e)| s >= e) {
        return None;
    }
    let mut luecken: Vec<(u32, u32)> = belegt
        .iter()
        .map(|(s, e)| ((*s).max(fenster.0), (*e).min(fenster.1)))
        .filter(|(s, e)| s < e)
        .collect();
    luecken.sort_unstable();
    let mut frei = Vec::new();
    let mut zeiger = fenster.0;
    let mut nimm = |von: u32, bis: u32| {
        if bis > von && bis - von >= mindestdauer.max(1) {
            frei.push((von, bis));
        }
    };
    for (s, e) in luecken {
        nimm(zeiger, s);
        zeiger = zeiger.max(e);
    }
    nimm(zeiger, fenster.1);
    Some(frei)
}

// ---------------------------------------------------------------- Empfehlung

const KENNZEICHNUNGEN: [&str; 6] = [
    "ich würde",
    "ich empfehle",
    "empfehlung",
    "i would",
    "i recommend",
    "recommendation",
];

/// Texte der Begruendungen samt ihrer Belege (Quellenauszuege, Eingaben, verschachtelte Gruende).
fn gruende_text(gruende: &[Satz], aus: &mut String) {
    for g in gruende {
        aus.push_str(&g.text);
        aus.push(' ');
        match &g.beleg {
            Some(Beleg::Quelle { auszug, .. }) => {
                aus.push_str(auszug);
                aus.push(' ');
            }
            Some(Beleg::Herleitung(
                Herleitung::Summe { eingaben_beleg, .. }
                | Herleitung::Produkt { eingaben_beleg, .. }
                | Herleitung::FreieZeit { eingaben_beleg, .. }
                | Herleitung::Frist { eingaben_beleg, .. },
            )) => {
                for (_, a) in eingaben_beleg {
                    aus.push_str(a);
                    aus.push(' ');
                }
            }
            Some(Beleg::Empfehlung { gruende }) => gruende_text(gruende, aus),
            _ => {}
        }
    }
}

fn pruefe_empfehlung(text: &str, gruende: &[Satz], ctx: &Ctx, tiefe: u8) -> Result<(), Grund> {
    let klein = text.to_lowercase();
    if !KENNZEICHNUNGEN.iter().any(|k| klein.contains(k)) {
        return Err(Grund::EmpfehlungOhneKennzeichnung);
    }
    if gruende.is_empty() {
        return Err(Grund::EmpfehlungOhneGrund);
    }
    for grund in gruende {
        // Ein Grund muss selbst ein belegter Aussagesatz sein.
        if grund.art != SatzArt::Aussage || pruefe_satz(grund, ctx, tiefe + 1).is_err() {
            return Err(Grund::BegruendungVerworfen);
        }
    }
    // Zahlen, Daten und Namen der Empfehlung selbst muessen in den Gruenden stehen.
    let mut korpus = String::new();
    gruende_text(gruende, &mut korpus);
    inhalt_gedeckt(text, &korpus)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ans() -> Ansagen {
        Ansagen::aus_tabelle(&Regeltabelle::standard())
    }

    fn merke(q: &mut QuellenMap, id: &str, auszug: &str) {
        let alt = q.text(id).map(|t| format!("{t} ")).unwrap_or_default();
        q.einfuegen(id, format!("{alt}{auszug}"));
    }

    /// Baut eine Quellenablage, in der jeder Auszug woertlich vorkommt.
    fn sammle(s: &Satz, q: &mut QuellenMap) {
        match &s.beleg {
            Some(Beleg::Quelle { quelle_id, auszug }) => merke(q, quelle_id, auszug),
            Some(Beleg::Empfehlung { gruende }) => gruende.iter().for_each(|g| sammle(g, q)),
            Some(Beleg::Herleitung(
                Herleitung::Summe { eingaben_beleg, .. }
                | Herleitung::Produkt { eingaben_beleg, .. }
                | Herleitung::FreieZeit { eingaben_beleg, .. }
                | Herleitung::Frist { eingaben_beleg, .. },
            )) => eingaben_beleg.iter().for_each(|(id, a)| merke(q, id, a)),
            _ => {}
        }
    }

    fn karte(s: &Satz) -> QuellenMap {
        let mut q = QuellenMap::new();
        sammle(s, &mut q);
        q
    }

    fn quelle(text: &str, auszug: &str) -> Satz {
        Satz {
            text: text.into(),
            art: SatzArt::Aussage,
            beleg: Some(Beleg::Quelle {
                quelle_id: "mail-1".into(),
                auszug: auszug.into(),
            }),
        }
    }

    fn herleitung(text: &str, h: Herleitung) -> Satz {
        Satz {
            text: text.into(),
            art: SatzArt::Aussage,
            beleg: Some(Beleg::Herleitung(h)),
        }
    }

    fn eing(auszug: &str) -> EingabenBeleg {
        vec![("e1".into(), auszug.into())]
    }

    fn grund_mit(s: Satz, q: &QuellenMap) -> Option<Grund> {
        pruefen(&[s], q, &ans()).verworfen.first().map(|(_, g)| *g)
    }

    fn grund_von(s: Satz) -> Option<Grund> {
        let q = karte(&s);
        grund_mit(s, &q)
    }

    fn summe(text: &str, ergebnis: i64) -> Satz {
        herleitung(
            text,
            Herleitung::Summe {
                werte: vec![Dezimal(100_050), Dezimal(23_400)],
                ergebnis: Dezimal(ergebnis),
                eingaben_beleg: eing("Posten 1000,50 Euro und 234,00 Euro"),
            },
        )
    }

    // ---- Grundfaelle

    #[test]
    fn rueckfragen_und_ansagen_ohne_beleg() {
        let r = pruefen(
            &[
                Satz {
                    text: "Meinst du Montag?".into(),
                    art: SatzArt::Rueckfrage,
                    beleg: None,
                },
                Satz {
                    text: "Ich schaue in deine Mails.".into(),
                    art: SatzArt::Ansage,
                    beleg: None,
                },
            ],
            &QuellenMap::new(),
            &ans(),
        );
        assert_eq!(r.ausgegeben.len(), 2);
        assert!(r.verworfen.is_empty());
    }

    #[test]
    fn aussage_ohne_beleg_verworfen() {
        let s = Satz {
            text: "Paris ist die Hauptstadt.".into(),
            art: SatzArt::Aussage,
            beleg: None,
        };
        assert_eq!(grund_von(s), Some(Grund::OhneBeleg));
    }

    // ---- 1 Nicht-Aussagen

    #[test]
    fn rueckfrage_ohne_fragezeichen_ist_aussage() {
        let s = Satz {
            text: "Der Vertrag kostet 500 Euro.".into(),
            art: SatzArt::Rueckfrage,
            beleg: None,
        };
        assert_eq!(grund_von(s), Some(Grund::OhneBeleg));
        let ok = Satz {
            text: "Meinst du Montag? ".into(),
            art: SatzArt::Rueckfrage,
            beleg: None,
        };
        assert_eq!(grund_von(ok), None);
    }

    #[test]
    fn ansage_nur_mit_bekannter_vorlage() {
        let ansage = |t: &str| Satz {
            text: t.into(),
            art: SatzArt::Ansage,
            beleg: None,
        };
        assert_eq!(
            grund_von(ansage("Der Vertrag kostet 5000 Euro.")),
            Some(Grund::OhneBeleg)
        );
        assert_eq!(grund_von(ansage("Let me think for a moment.")), None);
        assert_eq!(grund_von(ansage("Ich denke kurz nach.")), None);
        assert_eq!(grund_von(ansage(keine_quelle_angebot(Sprache::En))), None);
        assert_eq!(
            grund_von(ansage(&kein_beleg(Sprache::De, "deinen Mails"))),
            None
        );
        let schmuggel = "Dazu finde ich keinen Beleg in x. Der Vertrag kostet 5 Euro.";
        assert_eq!(grund_von(ansage(schmuggel)), Some(Grund::OhneBeleg));
    }

    // ---- Quelle

    #[test]
    fn quelle_passt() {
        let s = quelle(
            "Der Vertrag mit Müller GmbH läuft bis 12.03.2026 und kostet 1.234,50 Euro.",
            "Vertrag mit Müller GmbH läuft bis 2026-03-12 und kostet 1234.50 Euro",
        );
        assert_eq!(grund_von(s), None);
    }

    #[test]
    fn zahl_fehlt_in_quelle() {
        let s = quelle("Die Rechnung beträgt 480 Euro.", "Rechnung über 408 Euro");
        assert_eq!(grund_von(s), Some(Grund::ZahlFehltInQuelle));
    }

    #[test]
    fn datum_fehlt_in_quelle() {
        let s = quelle("Der Termin ist am 13.03.2026.", "Termin am 12.03.2026");
        assert_eq!(grund_von(s), Some(Grund::DatumFehltInQuelle));
    }

    #[test]
    fn name_fehlt_in_quelle() {
        let s = quelle("Die Mail kam von Schmidt.", "Mail von Meier zum Vertrag");
        assert_eq!(grund_von(s), Some(Grund::NameFehltInQuelle));
        let ok = quelle("Die Mail kam von Meiers Büro.", "Mail kam von Meiers Büro");
        assert_eq!(grund_von(ok), None);
    }

    #[test]
    fn verneinung_gekippt() {
        let s = quelle("Der Kunde hat zugesagt.", "Der Kunde hat nicht zugesagt.");
        assert_eq!(grund_von(s), Some(Grund::PolaritaetGekippt));
        let s = quelle("The client did not agree.", "The client agreed.");
        assert_eq!(grund_von(s), Some(Grund::PolaritaetGekippt));
        let s = quelle("The client didn't agree.", "The client agreed.");
        assert_eq!(grund_von(s), Some(Grund::PolaritaetGekippt));
        let ok = quelle("Es gibt keine Verzögerung.", "Es gibt keine Verzögerung.");
        assert_eq!(grund_von(ok), None);
    }

    #[test]
    fn leere_quelle() {
        let s = quelle("Der Termin steht.", "  ");
        assert_eq!(grund_von(s), Some(Grund::LeereQuelle));
    }

    // ---- 3 Zahlen

    #[test]
    fn zahlenparser_streng() {
        let w = |n: i128| vec![Zahl::Wert(n)];
        assert_eq!(parse_token("1.234,50"), w(1_234_500_000));
        assert_eq!(parse_token("1,234.5"), w(1_234_500_000));
        assert_eq!(parse_token("1234.50"), w(1_234_500_000));
        assert_eq!(parse_token("1234,5"), w(1_234_500_000));
        assert_eq!(parse_token("1,5"), w(1_500_000));
        assert_eq!(parse_token("0,500"), w(500_000));
        assert_eq!(parse_token("1.234.567"), w(1_234_567_000_000));
        assert_eq!(parse_token("12"), w(12_000_000));
        assert_eq!(parse_token("1.234"), vec![Zahl::Mehrdeutig]);
        assert_eq!(parse_token("1.500"), vec![Zahl::Mehrdeutig]);
        assert_eq!(
            parse_token("1,2,3"),
            vec![
                Zahl::Wert(1_000_000),
                Zahl::Wert(2_000_000),
                Zahl::Wert(3_000_000)
            ]
        );
        assert_eq!(parse_token("12.34.56").len(), 3);
        assert!(!parse_token("1,2,3").contains(&Zahl::Wert(123_000_000)));
    }

    #[test]
    fn faktor_1000_mehrdeutigkeit() {
        let s = quelle("Es sind 1,5 Stunden.", "Dauer 1.500 Stunden");
        assert_eq!(grund_von(s), Some(Grund::ZahlFehltInQuelle));
        let s = quelle("Es sind 1.500 Stunden.", "Dauer 1,5 Stunden");
        assert_eq!(grund_von(s), Some(Grund::ZahlMehrdeutig));
        let ok = quelle("Es sind 1,5 Stunden.", "Dauer 1,5 Stunden");
        assert_eq!(grund_von(ok), None);
    }

    #[test]
    fn ungueltige_gruppierung_ist_keine_zahl() {
        let s = quelle("Es sind 123 Stunden.", "Werte 1,2,3 Stunden");
        assert_eq!(grund_von(s), Some(Grund::ZahlFehltInQuelle));
        let s = quelle("Es sind 1.23 Stunden.", "Dauer 123 Stunden");
        assert_eq!(grund_von(s), Some(Grund::ZahlFehltInQuelle));
    }

    // ---- 4 Vorzeichen

    #[test]
    fn vorzeichen_in_der_quelle() {
        let s = quelle("Es sind -5 Grad.", "Temperatur 5 Grad");
        assert_eq!(grund_von(s), Some(Grund::ZahlFehltInQuelle));
        let s = quelle("Es sind 5 Grad.", "Temperatur \u{2212}5 Grad");
        assert_eq!(grund_von(s), Some(Grund::ZahlFehltInQuelle));
        let ok = quelle("Es sind -5 Grad.", "Temperatur \u{2212}5 Grad");
        assert_eq!(grund_von(ok), None);
        // Bereich "9-11" ist kein Vorzeichen.
        assert_eq!(
            zahlen_von("9-11"),
            vec![Zahl::Wert(9_000_000), Zahl::Wert(11_000_000)]
        );
    }

    #[test]
    fn vorzeichen_in_der_herleitung() {
        let h = |text: &str| {
            herleitung(
                text,
                Herleitung::Summe {
                    werte: vec![Dezimal(-2000), Dezimal(500)],
                    ergebnis: Dezimal(-1500),
                    eingaben_beleg: eing("Posten -20,00 Euro und 5,00 Euro"),
                },
            )
        };
        assert_eq!(grund_von(h("Zusammen sind das -15,00 Euro.")), None);
        assert_eq!(
            grund_von(h("Zusammen sind das 15,00 Euro.")),
            Some(Grund::UnbelegteZahlImSatz)
        );
    }

    // ---- 5 Namen

    #[test]
    fn erstes_wort_wird_als_name_geprueft() {
        let s = quelle("Schmidt hat zugesagt.", "Meier hat zugesagt.");
        assert_eq!(grund_von(s), Some(Grund::NameFehltInQuelle));
        let ok = quelle("Meier hat zugesagt.", "Meier hat zugesagt.");
        assert_eq!(grund_von(ok), None);
    }

    #[test]
    fn name_braucht_wortgrenze() {
        let s = quelle("Die Mail kam von Max.", "Mail von Maximilian");
        assert_eq!(grund_von(s), Some(Grund::NameFehltInQuelle));
        let s = quelle("Die Mail kam von Eva.", "Mail zur Evaluation");
        assert_eq!(grund_von(s), Some(Grund::NameFehltInQuelle));
        let ok = quelle("Die Mail kam von Max.", "Mail kam von Max");
        assert_eq!(grund_von(ok), None);
    }

    // ---- 6 Verneinung

    #[test]
    fn erweiterte_verneinungen() {
        for auszug in [
            "Der Vertrag gilt ohne Frist.",
            "Der Vertrag gilt weder heute noch morgen.",
            "Der Vertrag gilt nicht.",
            "Nein, der Vertrag gilt.",
            "Niemand sagt, der Vertrag gilt.",
        ] {
            let s = quelle("Der Vertrag gilt.", auszug);
            assert_eq!(grund_von(s), Some(Grund::PolaritaetGekippt), "{auszug}");
        }
        for auszug in [
            "The contract cannot apply.",
            "The contract applies without delay.",
            "Nobody says the contract applies.",
            "The contract won't apply.",
            "None of it applies.",
            "Neither party agrees.",
            "Nothing applies.",
        ] {
            let s = quelle("The contract applies.", auszug);
            assert_eq!(grund_von(s), Some(Grund::PolaritaetGekippt), "{auszug}");
        }
    }

    // ---- 8 Quellenbindung

    #[test]
    fn quelle_muss_existieren() {
        let s = quelle("Der Vertrag läuft bis Montag.", "Vertrag läuft bis Montag");
        assert_eq!(
            grund_mit(s, &QuellenMap::new()),
            Some(Grund::QuelleUnbekannt)
        );
    }

    #[test]
    fn auszug_muss_woertlich_in_der_quelle_stehen() {
        let q = QuellenMap::new().mit("mail-1", "Der Vertrag   läuft\nbis Montag.");
        let ok = quelle("Der Vertrag läuft bis Montag.", "Vertrag läuft bis Montag");
        assert_eq!(grund_mit(ok, &q), None);
        let erfunden = quelle("Der Vertrag läuft bis Montag.", "Vertrag läuft bis Freitag");
        assert_eq!(grund_mit(erfunden, &q), Some(Grund::AuszugNichtInQuelle));
        let umgestellt = quelle("Der Vertrag läuft bis Montag.", "bis Montag Vertrag läuft");
        assert_eq!(grund_mit(umgestellt, &q), Some(Grund::AuszugNichtInQuelle));
    }

    // ---- Herleitung

    #[test]
    fn summe_richtig_und_falsch() {
        assert_eq!(
            grund_von(summe("Zusammen sind das 1.234,50 Euro.", 123_450)),
            None
        );
        assert_eq!(
            grund_von(summe("Zusammen sind das 1234.50 Euro.", 123_450)),
            None
        );
        assert_eq!(
            grund_von(summe("Zusammen sind das viel Geld.", 123_450)),
            Some(Grund::ErgebnisNichtImSatz)
        );
        assert_eq!(
            grund_von(summe("Zusammen sind das 1.300,00 Euro.", 123_450)),
            Some(Grund::UnbelegteZahlImSatz)
        );
        assert_eq!(
            grund_von(summe("Zusammen sind das 1.300,00 Euro.", 130_000)),
            Some(Grund::RechnungFalsch)
        );
    }

    #[test]
    fn summe_mit_fremder_zahl_im_satz() {
        let s = summe("Zusammen sind das 1.234,50 Euro bei 7 Posten.", 123_450);
        assert_eq!(grund_von(s), Some(Grund::UnbelegteZahlImSatz));
        let s = summe("Zusammen sind das 1.234 Euro.", 123_450);
        assert_eq!(grund_von(s), Some(Grund::ZahlMehrdeutig));
    }

    #[test]
    fn operanden_brauchen_herkunft() {
        let ohne = herleitung(
            "Zusammen sind das 1.234,50 Euro.",
            Herleitung::Summe {
                werte: vec![Dezimal(100_050), Dezimal(23_400)],
                ergebnis: Dezimal(123_450),
                eingaben_beleg: vec![],
            },
        );
        assert_eq!(grund_von(ohne), Some(Grund::EingabenOhneBeleg));
        let luecke = herleitung(
            "Zusammen sind das 1.234,50 Euro.",
            Herleitung::Summe {
                werte: vec![Dezimal(100_050), Dezimal(23_400)],
                ergebnis: Dezimal(123_450),
                eingaben_beleg: eing("Posten 1000,50 Euro"),
            },
        );
        assert_eq!(grund_von(luecke), Some(Grund::OperandNichtBelegt));
        let q = QuellenMap::new().mit("e1", "Posten 1000,50 Euro und 234,00 Euro");
        let erfunden = herleitung(
            "Zusammen sind das 1.234,50 Euro.",
            Herleitung::Summe {
                werte: vec![Dezimal(100_050), Dezimal(23_400)],
                ergebnis: Dezimal(123_450),
                eingaben_beleg: eing("Posten 1000,50 Euro und 234,00 Euro und mehr"),
            },
        );
        assert_eq!(grund_mit(erfunden, &q), Some(Grund::AuszugNichtInQuelle));
    }

    #[test]
    fn produkt_stundensatz_mal_tage() {
        let p = |text: &str, ergebnis: i64| {
            herleitung(
                text,
                Herleitung::Produkt {
                    faktoren: vec![Dezimal(85_000), Dezimal(250)],
                    ergebnis: Dezimal(ergebnis),
                    eingaben_beleg: eing("Satz 850 Euro pro Tag, Aufwand 2,5 Tage"),
                },
            )
        };
        assert_eq!(
            grund_von(p("850 Euro mal 2,5 Tage sind 2125 Euro.", 212_500)),
            None
        );
        assert_eq!(
            grund_von(p("Das sind 2000 Euro.", 200_000)),
            Some(Grund::RechnungFalsch)
        );
        assert_eq!(
            grund_von(p("850 Euro mal 3 Tage sind 2125 Euro.", 212_500)),
            Some(Grund::UnbelegteZahlImSatz)
        );
    }

    /// Auszug, der alle Zeiten von `belegt` und `fenster` nennt.
    fn zeit_auszug(belegt: &[(u32, u32)], fenster: (u32, u32)) -> String {
        let hm = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
        let mut teile: Vec<String> = belegt
            .iter()
            .map(|(s, e)| format!("Termin {} bis {}", hm(*s), hm(*e)))
            .collect();
        teile.push(format!("Zeitraum {} bis {}", hm(fenster.0), hm(fenster.1)));
        teile.join(", ")
    }

    fn zeit(
        belegt: Vec<(u32, u32)>,
        fenster: (u32, u32),
        mind: u32,
        ergebnis: Vec<(u32, u32)>,
        text: &str,
    ) -> Satz {
        let eingaben = eing(&zeit_auszug(&belegt, fenster));
        zeit_mit(belegt, fenster, false, mind, ergebnis, text, eingaben)
    }

    fn zeit_mit(
        belegt: Vec<(u32, u32)>,
        fenster: (u32, u32),
        fenster_von_nutzer: bool,
        mind: u32,
        ergebnis: Vec<(u32, u32)>,
        text: &str,
        eingaben_beleg: EingabenBeleg,
    ) -> Satz {
        herleitung(
            text,
            Herleitung::FreieZeit {
                belegt,
                fenster,
                fenster_von_nutzer,
                mindestdauer: mind,
                ergebnis,
                eingaben_beleg,
            },
        )
    }

    #[test]
    fn freie_zeit_korrekt_und_falsch() {
        let belegt = vec![
            (9 * 60, 10 * 60),
            (11 * 60, 12 * 60 + 30),
            (10 * 60 + 30, 11 * 60 + 15),
        ];
        let erg = vec![(8 * 60, 9 * 60), (12 * 60 + 30, 14 * 60)];
        let text = "Frei bist du von 08:00 bis 09:00 Uhr und von 12:30 bis 14:00 Uhr.";
        assert_eq!(grund_von(zeit(belegt, (480, 840), 45, erg, text)), None);
        let s = zeit(
            vec![(540, 600)],
            (480, 840),
            45,
            vec![(480, 540)],
            "Frei von 08:00 bis 09:00.",
        );
        assert_eq!(grund_von(s), Some(Grund::RechnungFalsch));
        let s = zeit(
            vec![(540, 600)],
            (480, 660),
            30,
            vec![(480, 540), (600, 660)],
            "Frei von 08:00 bis 09:00.",
        );
        assert_eq!(grund_von(s), Some(Grund::ErgebnisNichtImSatz));
    }

    #[test]
    fn freie_zeit_englische_uhrzeit() {
        let s = zeit(
            vec![(540, 840)],
            (480, 900),
            30,
            vec![(480, 540), (840, 900)],
            "You are free 8 AM to 9 AM and 2:00 PM to 3 PM.",
        );
        assert_eq!(grund_von(s), None);
        assert_eq!(zeiten_im_text("12:05 AM und 12 PM"), vec![5, 720]);
    }

    #[test]
    fn freie_zeit_zusaetzliche_uhrzeit_verworfen() {
        let s = zeit(
            vec![(540, 600)],
            (480, 600),
            30,
            vec![(480, 540)],
            "Frei von 08:00 bis 09:00 und 11:00.",
        );
        assert_eq!(grund_von(s), Some(Grund::UnbelegteZahlImSatz));
    }

    #[test]
    fn freie_zeit_leeres_ergebnis_nur_mit_vorlage() {
        let leer = |t: &str| zeit(vec![(480, 840)], (480, 840), 30, vec![], t);
        assert_eq!(grund_von(leer(keine_freie_zeit(Sprache::De))), None);
        assert_eq!(grund_von(leer(keine_freie_zeit(Sprache::En))), None);
        assert_eq!(
            grund_von(leer("Du hast leider keinen Platz.")),
            Some(Grund::ErgebnisNichtImSatz)
        );
        assert_eq!(
            grund_von(leer(
                "Im Zeitraum gibt es keine freie Zeit, aber am Freitag."
            )),
            Some(Grund::ErgebnisNichtImSatz)
        );
    }

    #[test]
    fn freie_zeit_mit_verneinung_verworfen() {
        let s = zeit(
            vec![(540, 600)],
            (480, 600),
            30,
            vec![(480, 540)],
            "Frei von 08:00 bis 09:00, nicht aber später.",
        );
        assert_eq!(grund_von(s), Some(Grund::VerneinungUnzulaessig));
    }

    #[test]
    fn freie_zeit_braucht_eingaben_mit_herkunft() {
        let text = "Frei von 08:00 bis 09:00.";
        let s = zeit_mit(
            vec![(540, 600)],
            (480, 600),
            false,
            30,
            vec![(480, 540)],
            text,
            vec![],
        );
        assert_eq!(grund_von(s), Some(Grund::EingabenOhneBeleg));
        let s = zeit_mit(
            vec![(540, 600)],
            (480, 600),
            false,
            30,
            vec![(480, 540)],
            text,
            eing("Termin 09:00 bis 10:00, Zeitraum 08:00 bis 10:00"),
        );
        let q = QuellenMap::new().mit("e1", "Ganz anderer Text");
        assert_eq!(grund_mit(s, &q), Some(Grund::AuszugNichtInQuelle));
    }

    #[test]
    fn freie_zeit_falsche_eingaben_verworfen() {
        let text = "Frei von 08:00 bis 09:00.";
        let ergebnis = || vec![(480, 540)];
        // Belegtes Intervall steht nicht im Auszug.
        let s = zeit_mit(
            vec![(540, 600)],
            (480, 600),
            false,
            30,
            ergebnis(),
            text,
            eing("Termin 13:00 bis 14:00, Zeitraum 08:00 bis 10:00"),
        );
        assert_eq!(grund_von(s), Some(Grund::OperandNichtBelegt));
        // Nur der Start steht im Auszug.
        let s = zeit_mit(
            vec![(540, 600)],
            (480, 720),
            false,
            30,
            vec![(480, 540), (600, 720)],
            "Frei von 08:00 bis 09:00 und von 10:00 bis 12:00.",
            eing("Termin ab 09:00, Zeitraum 08:00 bis 12:00"),
        );
        assert_eq!(grund_von(s), Some(Grund::OperandNichtBelegt));
        // Start und Ende in verschiedenen Auszuegen zaehlen nicht.
        let s = zeit_mit(
            vec![(540, 600)],
            (480, 600),
            false,
            30,
            ergebnis(),
            text,
            vec![
                ("e1".into(), "Beginn 09:00".into()),
                ("e2".into(), "Ende 10:00, Zeitraum 08:00 bis 10:00".into()),
            ],
        );
        let q = QuellenMap::new()
            .mit("e1", "Beginn 09:00")
            .mit("e2", "Ende 10:00, Zeitraum 08:00 bis 10:00");
        assert_eq!(grund_mit(s, &q), Some(Grund::OperandNichtBelegt));
        // Fenster fehlt im Auszug und ist nicht vom Nutzer.
        let s = zeit_mit(
            vec![(540, 600)],
            (480, 600),
            false,
            30,
            ergebnis(),
            text,
            eing("Termin 09:00 bis 10:00"),
        );
        assert_eq!(grund_von(s), Some(Grund::OperandNichtBelegt));
        // Mit Nutzerfenster ist derselbe Auszug in Ordnung.
        let s = zeit_mit(
            vec![(540, 600)],
            (480, 600),
            true,
            30,
            ergebnis(),
            text,
            eing("Termin 09:00 bis 10:00"),
        );
        assert_eq!(grund_von(s), None);
    }

    #[test]
    fn freie_zeit_uhr_schreibweise_im_auszug() {
        let s = zeit_mit(
            vec![(540, 600)],
            (480, 600),
            false,
            30,
            vec![(480, 540)],
            "Frei von 08:00 bis 09:00.",
            eing("Besprechung 9 Uhr bis 10 Uhr, Zeitraum 8 Uhr bis 10 Uhr"),
        );
        assert_eq!(grund_von(s), None);
    }

    fn frist(text: &str, ergebnis: Datum) -> Satz {
        herleitung(
            text,
            Herleitung::Frist {
                start_tag: Datum {
                    jahr: 2026,
                    monat: 2,
                    tag: 20,
                },
                tage: 14,
                ergebnis,
                eingaben_beleg: eing("Beginn 20.02.2026, Frist 14 Tage"),
            },
        )
    }

    #[test]
    fn frist_rechnet_nach() {
        let ziel = Datum {
            jahr: 2026,
            monat: 3,
            tag: 6,
        };
        assert_eq!(
            grund_von(frist("Die Frist von 14 Tagen endet am 06.03.2026.", ziel)),
            None
        );
        assert_eq!(
            grund_von(frist("Die Frist von 14 Tagen endet am 6. März 2026.", ziel)),
            None
        );
        assert_eq!(
            grund_von(frist("The 14 day deadline is March 6, 2026.", ziel)),
            None
        );
        assert_eq!(
            grund_von(frist("Die Frist von 14 Tagen endet am 07.03.2026.", ziel)),
            Some(Grund::ErgebnisNichtImSatz)
        );
        let falsch = Datum {
            jahr: 2026,
            monat: 3,
            tag: 7,
        };
        assert_eq!(
            grund_von(frist("Die Frist von 14 Tagen endet am 07.03.2026.", falsch)),
            Some(Grund::RechnungFalsch)
        );
    }

    #[test]
    fn frist_muss_tage_nennen_und_belegt_sein() {
        let ziel = Datum {
            jahr: 2026,
            monat: 3,
            tag: 6,
        };
        assert_eq!(
            grund_von(frist("Die Frist endet am 06.03.2026.", ziel)),
            Some(Grund::TageNichtImSatz)
        );
        let ohne_start = herleitung(
            "Die Frist von 14 Tagen endet am 06.03.2026.",
            Herleitung::Frist {
                start_tag: Datum {
                    jahr: 2026,
                    monat: 2,
                    tag: 20,
                },
                tage: 14,
                ergebnis: ziel,
                eingaben_beleg: eing("Frist 14 Tage"),
            },
        );
        assert_eq!(grund_von(ohne_start), Some(Grund::OperandNichtBelegt));
    }

    #[test]
    fn datumsrechnung_schaltjahr() {
        let d = Datum {
            jahr: 2028,
            monat: 2,
            tag: 28,
        };
        assert_eq!(
            Datum::aus_tagen(d.tage_seit_epoche() + 1),
            Some(Datum {
                jahr: 2028,
                monat: 2,
                tag: 29
            })
        );
        assert_eq!(
            Datum {
                jahr: 1970,
                monat: 1,
                tag: 1
            }
            .tage_seit_epoche(),
            0
        );
        assert!(
            !Datum {
                jahr: 2026,
                monat: 2,
                tag: 29
            }
            .gueltig()
        );
    }

    // ---- Empfehlung

    fn empfehlung(text: &str, gruende: Vec<Satz>) -> Satz {
        Satz {
            text: text.into(),
            art: SatzArt::Aussage,
            beleg: Some(Beleg::Empfehlung { gruende }),
        }
    }

    #[test]
    fn empfehlung_mit_belegten_gruenden() {
        let grund = quelle(
            "Der Termin am 12.03.2026 ist frei.",
            "Termin im Kalender: 12.03.2026 frei",
        );
        assert_eq!(
            grund_von(empfehlung("Ich würde zusagen.", vec![grund])),
            None
        );
    }

    #[test]
    fn empfehlung_mit_unbelegtem_grund_verworfen() {
        let schlecht = Satz {
            text: "Das ist üblich.".into(),
            art: SatzArt::Aussage,
            beleg: None,
        };
        assert_eq!(
            grund_von(empfehlung("Ich würde zusagen.", vec![schlecht])),
            Some(Grund::BegruendungVerworfen)
        );
    }

    #[test]
    fn empfehlung_ohne_kennzeichnung_oder_grund() {
        let grund = quelle("Der Termin ist frei.", "Termin frei");
        assert_eq!(
            grund_von(empfehlung("Sag zu.", vec![grund])),
            Some(Grund::EmpfehlungOhneKennzeichnung)
        );
        assert_eq!(
            grund_von(empfehlung("I recommend accepting.", vec![])),
            Some(Grund::EmpfehlungOhneGrund)
        );
    }

    #[test]
    fn empfehlung_inhalt_muss_in_gruenden_stehen() {
        let grund = || quelle("Der Termin ist frei.", "Termin frei");
        assert_eq!(
            grund_von(empfehlung("Ich würde 500 Euro zusagen.", vec![grund()])),
            Some(Grund::ZahlFehltInQuelle)
        );
        assert_eq!(
            grund_von(empfehlung(
                "Ich würde am 13.03.2026 zusagen.",
                vec![grund()]
            )),
            Some(Grund::DatumFehltInQuelle)
        );
        assert_eq!(
            grund_von(empfehlung("Ich würde Schmidt zusagen.", vec![grund()])),
            Some(Grund::NameFehltInQuelle)
        );
    }

    // ---- Inhaltswoerter

    fn q(text: &str, auszug: &str) -> Option<Grund> {
        grund_von(quelle(text, auszug))
    }

    #[test]
    fn inhaltswort_zusage_gegen_absage() {
        assert_eq!(
            q("Der Kunde hat zugesagt.", "Der Kunde hat abgelehnt."),
            Some(Grund::InhaltswortFehlt)
        );
        assert_eq!(
            q("Der Kunde hat zugesagt.", "Der Kunde hat zugesagt."),
            None
        );
    }

    #[test]
    fn inhaltswort_bezahlt_gegen_offen() {
        assert_eq!(
            q("Die Rechnung ist bezahlt.", "Die Rechnung ist offen."),
            Some(Grund::InhaltswortFehlt)
        );
        assert_eq!(
            q("Die Rechnung ist offen.", "Die Rechnung ist bezahlt."),
            Some(Grund::InhaltswortFehlt)
        );
    }

    #[test]
    fn inhaltswort_englisch() {
        assert_eq!(
            q("The invoice was approved.", "The invoice was rejected."),
            Some(Grund::InhaltswortFehlt)
        );
        assert_eq!(
            q("The invoice was approved.", "The invoice was approved."),
            None
        );
        assert_eq!(q("Invoices approved.", "The invoice was approving."), None);
    }

    #[test]
    fn inhaltswort_flexion_wird_angenommen() {
        assert_eq!(
            q("Die Rechnungen wurden bezahlt.", "Rechnung bezahlt"),
            None
        );
        assert_eq!(
            q("Rechnung bezahlt.", "Die Rechnungen wurden bezahlt"),
            None
        );
        assert_eq!(q("Er hat das Angebot gesagt.", "sagen Angebote"), None);
    }

    #[test]
    fn inhaltswort_wortgrenze_statt_teilstring() {
        assert_eq!(
            q("The plan works.", "The plants work."),
            Some(Grund::InhaltswortFehlt)
        );
    }

    #[test]
    fn inhaltswort_funktionswoerter_und_kurze_woerter_zaehlen_nicht() {
        assert_eq!(q("Es ist ein Plan da.", "Plan"), None);
        assert_eq!(q("Wir haben es im Plan.", "Plan"), None);
    }

    #[test]
    fn inhaltswort_gilt_fuer_begruendungen_nicht_aber_fuer_empfehlungssatz_regel() {
        let falsch = quelle("Der Kunde hat zugesagt.", "Der Kunde hat abgelehnt.");
        assert_eq!(
            grund_von(empfehlung("Ich würde zusagen.", vec![falsch])),
            Some(Grund::BegruendungVerworfen)
        );
        // Kennzeichnungswoerter und freie Wortwahl im Empfehlungssatz bleiben erlaubt.
        let grund = quelle("Der Termin ist frei.", "Termin frei");
        assert_eq!(
            grund_von(empfehlung(
                "Ich empfehle, den Termin zu nehmen.",
                vec![grund]
            )),
            None
        );
    }

    #[test]
    fn herleitung_name_ausserhalb_der_auszuege_verworfen() {
        let s = summe("Die Rechnung von Meier beträgt 1234,50 Euro.", 123_450);
        assert_eq!(grund_von(s), Some(Grund::InhaltswortFehlt));
        let s = summe("Die Posten ergeben zusammen 1234,50 Euro.", 123_450);
        assert_eq!(grund_von(s), Some(Grund::InhaltswortFehlt));
        let s = zeit(
            vec![(540, 600)],
            (480, 600),
            30,
            vec![(480, 540)],
            "Bei Schmidt bist du frei von 08:00 bis 09:00.",
        );
        assert_eq!(grund_von(s), Some(Grund::InhaltswortFehlt));
        let s = frist(
            "Die Kündigung läuft bis 06.03.2026, also 14 Tage.",
            Datum {
                jahr: 2026,
                monat: 3,
                tag: 6,
            },
        );
        assert_eq!(grund_von(s), Some(Grund::InhaltswortFehlt));
    }

    #[test]
    fn herleitung_rahmen_und_auszugswoerter_je_art() {
        let s = summe("Die Summe beträgt 1234,50 Euro.", 123_450);
        assert_eq!(grund_von(s), None);
        let s = summe("Die Posten zusammen: 1234,50 Euro.", 123_450);
        assert_eq!(grund_von(s), None);
        let s = frist(
            "Die Frist endet am 06.03.2026, 14 Tage ab Beginn.",
            Datum {
                jahr: 2026,
                monat: 3,
                tag: 6,
            },
        );
        assert_eq!(grund_von(s), None);
        let s = zeit(
            vec![(540, 600)],
            (480, 600),
            30,
            vec![(480, 540)],
            "Am Montag bist du frei von 08:00 bis 09:00 Uhr.",
        );
        assert_eq!(grund_von(s), None);
    }

    #[test]
    fn stamm_regeln() {
        assert_eq!(flexion_ab("rechnungen"), "rechnung");
        assert_eq!(flexion_ab("bezahlt"), "bezahl");
        assert_eq!(flexion_ab("approved"), "approv");
        assert_eq!(flexion_ab("approving"), "approv");
        assert_eq!(flexion_ab("planet"), "plan");
        assert_eq!(flexion_ab("sie"), "sie");
        assert!(stammformen("gesagt").contains(&"sag".to_owned()));
        assert_eq!(
            stammformen("geld"),
            vec!["geld".to_owned(), "geld".to_owned()]
        );
    }

    #[test]
    fn angebotstexte() {
        assert_eq!(
            keine_quelle_angebot(Sprache::De),
            "Dazu habe ich keine Quelle. Soll ich recherchieren?"
        );
        assert_eq!(
            keine_quelle_angebot(Sprache::En),
            "I don't have a source for that. Shall I research it?"
        );
        assert_eq!(
            kein_beleg(Sprache::De, "deinen Mails"),
            "Dazu finde ich keinen Beleg in deinen Mails."
        );
        assert_eq!(
            kein_beleg(Sprache::En, "your emails"),
            "I can't find any evidence for that in your emails."
        );
    }

    #[test]
    fn rueckfrage_mit_eingeschmuggelter_aussage_wird_verworfen() {
        assert!(ist_einzelne_frage("Wie meinst du das?"));
        assert!(!ist_einzelne_frage(
            "Der Vertrag laeuft bis 2030. Wie meinst du das?"
        ));
        assert!(!ist_einzelne_frage("Das stimmt! Oder?"));
        assert!(!ist_einzelne_frage("Der Vertrag laeuft bis 2030"));
        assert!(!ist_einzelne_frage("?"));
    }

    // ---- Zusammenhaengende Teilaussagen

    fn gebunden(text: &str, auszug: &str) -> Option<Grund> {
        grund_von(quelle(text, auszug))
    }

    #[test]
    fn gegenbeispiel_beide_richtungen_de() {
        let a = "Der Kunde hat nicht zugesagt und der Lieferant hat zugesagt.";
        assert_eq!(gebunden("Der Kunde hat nicht zugesagt.", a), None);
        assert_eq!(gebunden("Der Lieferant hat zugesagt.", a), None);
        assert!(gebunden("Der Lieferant hat nicht zugesagt.", a).is_some());
        assert!(gebunden("Der Kunde hat zugesagt.", a).is_some());
    }

    #[test]
    fn gegenbeispiel_en() {
        let a = "The customer did not accept and the supplier accepted.";
        assert_eq!(gebunden("The customer did not accept.", a), None);
        assert_eq!(gebunden("The supplier accepted.", a), None);
        assert!(gebunden("The supplier did not accept.", a).is_some());
    }

    #[test]
    fn zwei_teilaussagen_aus_zwei_quellsaetzen() {
        let a = "Der Kunde hat zugesagt. Der Lieferant hat nicht geliefert.";
        let s = quelle(
            "Der Kunde hat zugesagt und der Lieferant hat nicht geliefert.",
            a,
        );
        assert_eq!(grund_von(s), None);
        let s = quelle(
            "Der Kunde hat nicht zugesagt und der Lieferant hat nicht geliefert.",
            a,
        );
        assert!(grund_von(s).is_some());
    }

    #[test]
    fn vermischung_zweier_teilaussagen_verworfen() {
        let a = "Der Kunde hat Angebot geprüft und der Lieferant hat Rechnung gestellt.";
        let s = quelle("Der Kunde hat Rechnung gestellt.", a);
        assert_eq!(grund_von(s), Some(Grund::KeineZusammenhaengendeAussage));
    }

    #[test]
    fn subjektlose_teilaussage_verworfen() {
        let a = "Der Kunde hat nicht zugesagt und der Lieferant hat zugesagt.";
        let s = quelle("Der Lieferant hat zugesagt und hat nicht zugesagt.", a);
        assert!(grund_von(s).is_some());
        let s = quelle("Der Kunde hat nicht zugesagt und hat nicht zugesagt.", a);
        assert_eq!(grund_von(s), Some(Grund::KeineZusammenhaengendeAussage));
    }

    #[test]
    fn doppelte_verneinung_verworfen() {
        let a = "Der Kunde hat kein Angebot nicht angenommen.";
        let s = quelle("Der Kunde hat kein Angebot nicht angenommen.", a);
        assert_eq!(grund_von(s), Some(Grund::KeineZusammenhaengendeAussage));
    }

    #[test]
    fn datumspunkt_trennt_nicht() {
        let a = "Der Termin ist am 12. März 2026 und der Preis beträgt 3,5 Euro.";
        let s = quelle("Der Termin ist am 12. März 2026.", a);
        assert_eq!(grund_von(s), None);
    }
}
