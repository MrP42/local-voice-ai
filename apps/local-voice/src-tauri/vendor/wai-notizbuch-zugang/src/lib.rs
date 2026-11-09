//! Nur lesender Zugang zur Ablage des WAI Notizbuchs für den Assistenten (lva#396, Stufe N0).
//!
//! Öffnet `notizbuch.db` (Notizbuch-Host: `<Datenordner>/de.wolffappliedai.notizbuch/notizbuch.db`)
//! **nur lesend** (`SQLITE_OPEN_READ_ONLY`, `query_only`) und ohne Migration. Passt die Schema-Version
//! nicht, liefert [`Zugang::oeffnen`] [`Fehler::Version`] statt am Schema zu raten. Geschrieben wird
//! nie direkt, sondern über den Eingang ([`eingang`], Spec §4.1).
//!
//! Sichtbarkeit (gleiche Regel wie der Wissens-Endpunkt `notizbuch-store::wissen`): Eine Seite ist
//! sichtbar, wenn sie, ihr Abschnitt, dessen Gruppe und Notizbuch existieren und nicht im Papierkorb
//! liegen **und** der Abschnitt in der [`Freigabe`] steht. Nicht freigegeben und nicht vorhanden
//! ergeben dieselbe Antwort (`None` bzw. kein Treffer).
//!
//! Inhalte gehen nie ins Log; Fehler tragen nur Codes.

pub mod bloecke;
pub mod eingang;

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;

pub use bloecke::{Block, BlockArt};

/// Schema-Versionen der Notizbuch-Ablage (`_wai_migrations`), die dieser Zugang lesen kann.
pub const SCHEMA_VERSIONEN: &[i64] = &[2, 3];
/// Höchstzahl Treffer und Zeichen je Seite (wie der Wissens-Endpunkt).
pub const MAX_TREFFER: usize = 20;
pub const MAX_TEXT: usize = 32_000;
/// Längste Suchanfrage (Zeichen) und höchste Zahl Wörter.
pub const MAX_ANFRAGE_ZEICHEN: usize = 200;
pub const MAX_ANFRAGE_WOERTER: usize = 10;
/// Länge des Auszugs in Treffern.
const AUSZUG: usize = 240;
/// Bis hierher (Zeichen) wird für den Auszug nach dem Suchwort gesucht.
const AUSZUG_SUCHFENSTER: usize = 100_000;

#[derive(Debug, thiserror::Error)]
pub enum Fehler {
    #[error("notizbuch.fehlt")]
    Fehlt,
    #[error("notizbuch.version")]
    Version,
    #[error("notizbuch.eingabe")]
    Eingabe,
    #[error("notizbuch.lesen")]
    Lesen,
}

impl From<rusqlite::Error> for Fehler {
    fn from(_: rusqlite::Error) -> Self {
        Fehler::Lesen
    }
}

pub type Ergebnis<T> = Result<T, Fehler>;

/// Ids im Notizbuch: kurze Bezeichner ohne Sonderzeichen (wie `notizbuch-store::gueltige_id`).
pub fn gueltige_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Faltung wie `falte()` im Notizbuch (Frontend und Store): klein, ohne Akzente, ß → ss.
/// Muss mit `notizbuch-store::falte` übereinstimmen, sonst findet die Suche im Index nichts.
pub fn falte(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let basis = match c {
            'ß' | 'ẞ' => {
                out.push_str("ss");
                continue;
            }
            'æ' | 'Æ' => {
                out.push_str("ae");
                continue;
            }
            'œ' | 'Œ' => {
                out.push_str("oe");
                continue;
            }
            'à'..='å' | 'À'..='Å' | 'ā' | 'Ā' | 'ă' | 'Ă' | 'ą' | 'Ą' => 'a',
            'ç' | 'Ç' | 'ć' | 'Ć' | 'č' | 'Č' => 'c',
            'ď' | 'Ď' => 'd',
            'è'..='ë' | 'È'..='Ë' | 'ē' | 'Ē' | 'ė' | 'Ė' | 'ę' | 'Ę' | 'ě' | 'Ě' => {
                'e'
            }
            'ì'..='ï' | 'Ì'..='Ï' | 'ī' | 'Ī' | 'į' | 'Į' => 'i',
            'ł' | 'Ł' | 'ľ' | 'Ľ' => 'l',
            'ñ' | 'Ñ' | 'ń' | 'Ń' | 'ň' | 'Ň' => 'n',
            'ò'..='ö' | 'Ò'..='Ö' | 'ø' | 'Ø' | 'ō' | 'Ō' | 'ő' | 'Ő' => 'o',
            'ř' | 'Ř' => 'r',
            'ś' | 'Ś' | 'š' | 'Š' | 'ş' | 'Ş' => 's',
            'ť' | 'Ť' | 'ţ' | 'Ţ' => 't',
            'ù'..='ü' | 'Ù'..='Ü' | 'ū' | 'Ū' | 'ů' | 'Ů' | 'ű' | 'Ű' => 'u',
            'ý' | 'ÿ' | 'Ý' | 'Ÿ' => 'y',
            'ź' | 'Ź' | 'ż' | 'Ż' | 'ž' | 'Ž' => 'z',
            _ => {
                out.extend(c.to_lowercase());
                continue;
            }
        };
        out.push(basis);
    }
    out
}

/// FTS5-Anfrage wie `notizbuch-store::fts_anfrage`: jedes Wort als Zeichenkette, alle per UND.
/// `None`, wenn ein Wort kürzer als drei Zeichen ist (Trigramm-Index trägt dann nicht).
pub fn fts_anfrage(anfrage: &str) -> Option<String> {
    let gefaltet = falte(anfrage);
    let woerter: Vec<&str> = gefaltet.split_whitespace().collect();
    if woerter.is_empty() || woerter.iter().any(|w| w.chars().count() < 3) {
        return None;
    }
    Some(
        woerter
            .iter()
            .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}

/// Freigegebene Abschnitte (für Lesen). Leer = nichts sichtbar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Freigabe {
    abschnitte: BTreeSet<String>,
    notizbuecher: BTreeSet<String>,
}

impl Freigabe {
    /// Einzelne Abschnitte.
    pub fn abschnitt(mut self, id: &str) -> Self {
        if gueltige_id(id) {
            self.abschnitte.insert(id.to_string());
        }
        self
    }
    /// Ganzes Notizbuch: alle heutigen und künftigen Abschnitte darin.
    pub fn notizbuch(mut self, id: &str) -> Self {
        if gueltige_id(id) {
            self.notizbuecher.insert(id.to_string());
        }
        self
    }
    pub fn ist_leer(&self) -> bool {
        self.abschnitte.is_empty() && self.notizbuecher.is_empty()
    }
    fn erlaubt(&self, a: &Abschnitt) -> bool {
        self.abschnitte.contains(&a.id) || self.notizbuecher.contains(&a.notizbuch_id)
    }
}

/// Knoten der Gliederung (Notizbuch, Gruppe, Abschnitt) für die Freigabe-Einstellung.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Notizbuch {
    pub id: String,
    pub name: String,
    pub abschnitte: Vec<Abschnitt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Abschnitt {
    pub id: String,
    pub name: String,
    pub notizbuch_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gruppe: Option<String>,
    /// Zahl aktiver Seiten.
    pub seiten: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Seitenkopf {
    pub id: String,
    pub titel: String,
    pub abschnitt_id: String,
    /// „Notizbuch › Gruppe › Abschnitt“.
    pub pfad: String,
    pub erstellt: String,
    pub geaendert: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Seite {
    #[serde(flatten)]
    pub kopf: Seitenkopf,
    pub bloecke: Vec<Block>,
    /// Klartext höchstens [`MAX_TEXT`] Zeichen.
    pub text: String,
    /// `text` wurde auf [`MAX_TEXT`] gekürzt (Werkzeugausgabe); `bloecke` sind dann noch vollständig.
    pub gekuerzt: bool,
    /// Die Blockliste erreichte [`bloecke::MAX_BLOECKE`]: die Seite ist nicht vollständig gelesen (Spec §5.4).
    pub unvollstaendig: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Treffer {
    #[serde(flatten)]
    pub kopf: Seitenkopf,
    pub auszug: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suche {
    /// `false`: Anfrage für den Index zu kurz (Wort unter drei Zeichen); dann keine Treffer.
    pub genutzt: bool,
    pub treffer: Vec<Treffer>,
}

/// Lesender Zugang.
pub struct Zugang {
    conn: Connection,
}

fn str_feld(v: &Value, f: &str) -> String {
    bloecke::sauber(v.get(f).and_then(Value::as_str).unwrap_or_default())
}

/// ISO-Zeit `YYYY-MM-DDTHH:MM:SS[.f…]Z` in die feste Form `YYYY-MM-DDTHH:MM:SS.mmmZ` (vergleichbar).
fn iso_norm(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let ziffer = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    let form = b.len() >= 20
        && [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18]
            .iter()
            .all(|&i| ziffer(i))
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':'
        && *b.last()? == b'Z';
    if !form {
        return None;
    }
    let rest = &s[19..s.len() - 1];
    let bruch = match rest.strip_prefix('.') {
        None if rest.is_empty() => "",
        Some(f) if !f.is_empty() && f.len() <= 9 && f.bytes().all(|c| c.is_ascii_digit()) => f,
        _ => return None,
    };
    let mut ms: String = bruch.chars().take(3).collect();
    while ms.len() < 3 {
        ms.push('0');
    }
    Some(format!("{}.{ms}Z", &s[..19]))
}

fn lebt(v: &Value) -> bool {
    v.get("geloescht").is_none_or(Value::is_null)
}

/// Aktive Gliederung, einmal je Aufruf aus der Ablage gelesen.
struct Gliederung {
    notizbuecher: HashMap<String, Value>,
    gruppen: HashMap<String, Value>,
    abschnitte: HashMap<String, Value>,
}

impl Gliederung {
    /// Abschnitt, wenn er samt Gruppe und Notizbuch aktiv ist.
    fn abschnitt(&self, id: &str) -> Option<Abschnitt> {
        let a = self.abschnitte.get(id)?;
        let nb = str_feld(a, "notizbuchId");
        self.notizbuecher.get(&nb)?;
        let gruppe = match a.get("gruppeId").and_then(Value::as_str) {
            Some(g) => Some(str_feld(self.gruppen.get(g)?, "name")),
            None => None,
        };
        Some(Abschnitt {
            id: id.to_string(),
            name: str_feld(a, "name"),
            notizbuch_id: nb,
            gruppe,
            seiten: 0,
        })
    }

    fn pfad(&self, a: &Abschnitt) -> String {
        let nb = self
            .notizbuecher
            .get(&a.notizbuch_id)
            .map(|v| str_feld(v, "name"))
            .unwrap_or_default();
        match &a.gruppe {
            Some(g) => format!("{nb} › {g} › {}", a.name),
            None => format!("{nb} › {}", a.name),
        }
    }
}

fn kuerzen(s: &str, max: usize) -> (String, bool) {
    if s.chars().count() <= max {
        (s.to_string(), false)
    } else {
        (s.chars().take(max).collect(), true)
    }
}

impl Zugang {
    /// Öffnet die Ablage nur lesend und prüft die Schema-Version.
    pub fn oeffnen(pfad: impl AsRef<Path>) -> Ergebnis<Self> {
        let pfad = pfad.as_ref();
        if !pfad.is_file() {
            return Err(Fehler::Fehlt);
        }
        let conn = Connection::open_with_flags(
            pfad,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "query_only", true)?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        let v: Option<i64> = conn
            .query_row("SELECT MAX(version) FROM _wai_migrations", [], |r| r.get(0))
            .map_err(|_| Fehler::Version)?;
        if !v.is_some_and(|v| SCHEMA_VERSIONEN.contains(&v)) {
            return Err(Fehler::Version);
        }
        Ok(Self { conn })
    }

    fn saetze(&self, art: &str) -> Ergebnis<HashMap<String, Value>> {
        let mut st = self
            .conn
            .prepare_cached("SELECT id, daten FROM saetze WHERE art = ?1")?;
        let mut out = HashMap::new();
        let mut rows = st.query(params![art])?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let daten: String = r.get(1)?;
            if let Ok(v) = serde_json::from_str::<Value>(&daten)
                && lebt(&v)
            {
                out.insert(id, v);
            }
        }
        Ok(out)
    }

    /// Seitenköpfe ohne Inhalt (nur die Felder für Gliederung, Stand und Zeitraum), lebende Seiten.
    fn seitenkoepfe(&self) -> Ergebnis<Vec<Value>> {
        let mut st = self.conn.prepare_cached(
            "SELECT id, json_extract(daten, '$.abschnittId'), json_extract(daten, '$.titel'), \
             json_extract(daten, '$.erstellt'), json_extract(daten, '$.geaendert'), \
             json_extract(daten, '$.version') FROM saetze \
             WHERE art = 'seiten' AND json_valid(daten) AND json_extract(daten, '$.geloescht') IS NULL",
        )?;
        let zeilen = st.query_map([], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, String>(0)?,
                "abschnittId": r.get::<_, Option<String>>(1).unwrap_or(None),
                "titel": r.get::<_, Option<String>>(2).unwrap_or(None),
                "erstellt": r.get::<_, Option<String>>(3).unwrap_or(None),
                "geaendert": r.get::<_, Option<String>>(4).unwrap_or(None),
                "version": r.get::<_, Option<i64>>(5).unwrap_or(None),
            }))
        })?;
        Ok(zeilen.collect::<Result<_, _>>()?)
    }

    /// Führt `f` in einer Lesetransaktion aus: alle Abfragen eines Aufrufs sehen denselben Stand.
    fn lesend<T>(&self, f: impl FnOnce() -> Ergebnis<T>) -> Ergebnis<T> {
        self.conn.execute_batch("BEGIN DEFERRED")?;
        let r = f();
        let ende = self
            .conn
            .execute_batch(if r.is_ok() { "COMMIT" } else { "ROLLBACK" });
        let r = r?;
        ende?;
        Ok(r)
    }

    fn gliederung(&self) -> Ergebnis<Gliederung> {
        Ok(Gliederung {
            notizbuecher: self.saetze("notizbuecher")?,
            gruppen: self.saetze("gruppen")?,
            abschnitte: self.saetze("abschnitte")?,
        })
    }

    fn seite_roh(&self, id: &str) -> Ergebnis<Option<Value>> {
        let d: Option<String> = self
            .conn
            .query_row(
                "SELECT daten FROM saetze WHERE art = 'seiten' AND id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(d.and_then(|d| serde_json::from_str::<Value>(&d).ok())
            .filter(lebt))
    }

    fn kopf(&self, g: &Gliederung, f: &Freigabe, s: &Value) -> Option<Seitenkopf> {
        let a = g.abschnitt(s.get("abschnittId")?.as_str()?)?;
        if !f.erlaubt(&a) {
            return None;
        }
        Some(Seitenkopf {
            id: str_feld(s, "id"),
            titel: str_feld(s, "titel"),
            abschnitt_id: a.id.clone(),
            pfad: g.pfad(&a),
            erstellt: str_feld(s, "erstellt"),
            geaendert: str_feld(s, "geaendert"),
        })
    }

    /// Freigabestand (Spec §3.2, Codex M2): Hash über die Freigabe und den Gliederungszustand aller
    /// sichtbaren Abschnitte und Seiten (Notizbuch, Gruppe, Abschnitt, Papierkorb, Stand). Papierkorb und
    /// Verschieben ändern ihn auch ohne neuen Seitenzeitstempel; ein anderer Stand verwirft Caches.
    pub fn stand(&self, f: &Freigabe) -> Ergebnis<String> {
        self.lesend(|| self.stand_innen(f))
    }

    fn stand_innen(&self, f: &Freigabe) -> Ergebnis<String> {
        use sha2::{Digest, Sha256};
        let g = self.gliederung()?;
        let mut zeilen: Vec<String> = Vec::new();
        for a in &f.abschnitte {
            zeilen.push(format!("fa\u{1f}{a}"));
        }
        for n in &f.notizbuecher {
            zeilen.push(format!("fn\u{1f}{n}"));
        }
        for id in g.abschnitte.keys() {
            if let Some(a) = g.abschnitt(id).filter(|a| f.erlaubt(a)) {
                zeilen.push(format!(
                    "a\u{1f}{}\u{1f}{}\u{1f}{:?}\u{1f}{}",
                    a.id,
                    a.notizbuch_id,
                    a.gruppe,
                    g.pfad(&a)
                ));
            }
        }
        for s in &self.seitenkoepfe()? {
            if let Some(k) = self.kopf(&g, f, s) {
                let v = s.get("version").and_then(Value::as_i64).unwrap_or(0);
                zeilen.push(format!(
                    "s\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{v}\u{1f}{}",
                    k.id, k.abschnitt_id, k.geaendert, k.titel
                ));
            }
        }
        zeilen.sort();
        let mut h = Sha256::new();
        for z in &zeilen {
            h.update(z.as_bytes());
            h.update(b"\n");
        }
        Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }

    /// Gliederung der **freigegebenen** Abschnitte mit Seitenzahl.
    pub fn baum(&self, f: &Freigabe) -> Ergebnis<Vec<Notizbuch>> {
        self.lesend(|| self.baum_mit(|a| f.erlaubt(a)))
    }

    /// Ganze aktive Gliederung, nur Namen und Zählung, für die Freigabe-Einstellung (kein Seiteninhalt).
    pub fn gliederung_fuer_einstellung(&self) -> Ergebnis<Vec<Notizbuch>> {
        self.lesend(|| self.baum_mit(|_| true))
    }

    fn baum_mit(&self, sichtbar: impl Fn(&Abschnitt) -> bool) -> Ergebnis<Vec<Notizbuch>> {
        let g = self.gliederung()?;
        let mut zahl: HashMap<String, usize> = HashMap::new();
        for s in &self.seitenkoepfe()? {
            *zahl.entry(str_feld(s, "abschnittId")).or_default() += 1;
        }
        let mut nbs: Vec<Notizbuch> = g
            .notizbuecher
            .iter()
            .map(|(id, v)| Notizbuch {
                id: id.clone(),
                name: str_feld(v, "name"),
                abschnitte: vec![],
            })
            .collect();
        let mut abschnitte: Vec<Abschnitt> = g
            .abschnitte
            .keys()
            .filter_map(|id| g.abschnitt(id))
            .filter(|a| sichtbar(a))
            .collect();
        abschnitte.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        for mut a in abschnitte {
            a.seiten = zahl.get(&a.id).copied().unwrap_or(0);
            if let Some(nb) = nbs.iter_mut().find(|n| n.id == a.notizbuch_id) {
                nb.abschnitte.push(a);
            }
        }
        nbs.retain(|n| !n.abschnitte.is_empty());
        nbs.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        Ok(nbs)
    }

    /// Eine Seite mit Blöcken; `None`, wenn nicht vorhanden, im Papierkorb oder nicht freigegeben.
    pub fn seite(&self, id: &str, f: &Freigabe) -> Ergebnis<Option<Seite>> {
        if !gueltige_id(id) {
            return Err(Fehler::Eingabe);
        }
        self.lesend(|| self.seite_innen(id, f))
    }

    fn seite_innen(&self, id: &str, f: &Freigabe) -> Ergebnis<Option<Seite>> {
        let Some(roh) = self.seite_roh(id)? else {
            return Ok(None);
        };
        let g = self.gliederung()?;
        let Some(kopf) = self.kopf(&g, f, &roh) else {
            return Ok(None);
        };
        let bloecke = roh.get("inhalt").map(bloecke::bloecke).unwrap_or_default();
        let unvollstaendig = bloecke.len() >= bloecke::MAX_BLOECKE;
        let voll = bloecke::klartext(&bloecke, MAX_TEXT + 1);
        let (text, gekuerzt) = kuerzen(&voll, MAX_TEXT);
        Ok(Some(Seite {
            kopf,
            bloecke,
            text,
            gekuerzt,
            unvollstaendig,
        }))
    }

    /// Volltextsuche (FTS5, Titeltreffer zuerst) in freigegebenen Abschnitten.
    pub fn suchen(&self, anfrage: &str, grenze: usize, f: &Freigabe) -> Ergebnis<Suche> {
        if anfrage.chars().count() > MAX_ANFRAGE_ZEICHEN
            || anfrage.split_whitespace().count() > MAX_ANFRAGE_WOERTER
        {
            return Err(Fehler::Eingabe);
        }
        let grenze = grenze.clamp(1, MAX_TREFFER);
        let Some(fts) = fts_anfrage(anfrage) else {
            return Ok(Suche {
                genutzt: false,
                treffer: vec![],
            });
        };
        if f.ist_leer() {
            return Ok(Suche {
                genutzt: true,
                treffer: vec![],
            });
        }
        self.lesend(|| self.suchen_innen(anfrage, &fts, grenze, f))
    }

    fn suchen_innen(
        &self,
        anfrage: &str,
        fts: &str,
        grenze: usize,
        f: &Freigabe,
    ) -> Ergebnis<Suche> {
        let g = self.gliederung()?;
        // Freigabefilter schon in SQL, damit nicht freigegebene Treffer keine Plätze belegen.
        let erlaubt: Vec<&String> = g
            .abschnitte
            .keys()
            .filter(|id| g.abschnitt(id).is_some_and(|a| f.erlaubt(&a)))
            .collect();
        let erlaubt = serde_json::to_string(&erlaubt).unwrap_or_else(|_| "[]".into());
        let mut st = self.conn.prepare_cached(
            "SELECT seiten_fts.id FROM seiten_fts JOIN saetze s ON s.art = 'seiten' AND s.id = seiten_fts.id \
             WHERE seiten_fts MATCH ?1 AND json_valid(s.daten) AND json_extract(s.daten, '$.geloescht') IS NULL \
             AND json_extract(s.daten, '$.abschnittId') IN (SELECT value FROM json_each(?2)) \
             ORDER BY bm25(seiten_fts, 0.0, 10.0, 1.0) LIMIT ?3",
        )?;
        let ids: Vec<String> = st
            .query_map(params![fts, erlaubt, (grenze * 4) as i64], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let woerter: Vec<String> = falte(anfrage)
            .split_whitespace()
            .map(String::from)
            .collect();
        let mut treffer = Vec::new();
        for id in ids {
            if treffer.len() >= grenze {
                break;
            }
            let Some(roh) = self.seite_roh(&id)? else {
                continue;
            };
            let Some(kopf) = self.kopf(&g, f, &roh) else {
                continue;
            };
            let text = roh.get("text").and_then(Value::as_str).unwrap_or_default();
            treffer.push(Treffer {
                kopf,
                auszug: auszug(text, &woerter),
            });
        }
        Ok(Suche {
            genutzt: true,
            treffer,
        })
    }

    /// Seiten, die im Zeitraum `[von, bis)` angelegt oder geändert wurden (ISO-Zeiten, UTC `Z`),
    /// neueste zuerst, höchstens `grenze`.
    pub fn zeitraum(
        &self,
        von: &str,
        bis: &str,
        grenze: usize,
        f: &Freigabe,
    ) -> Ergebnis<Vec<Seitenkopf>> {
        let (Some(von), Some(bis)) = (iso_norm(von), iso_norm(bis)) else {
            return Err(Fehler::Eingabe);
        };
        if von >= bis {
            return Err(Fehler::Eingabe);
        }
        self.lesend(|| self.zeitraum_innen(&von, &bis, grenze, f))
    }

    fn zeitraum_innen(
        &self,
        von: &str,
        bis: &str,
        grenze: usize,
        f: &Freigabe,
    ) -> Ergebnis<Vec<Seitenkopf>> {
        let g = self.gliederung()?;
        let mut out: Vec<Seitenkopf> = self
            .seitenkoepfe()?
            .iter()
            .filter_map(|s| self.kopf(&g, f, s))
            .filter(|k| {
                let e = iso_norm(&k.erstellt);
                let a = iso_norm(&k.geaendert);
                let drin = |t: &Option<String>| t.as_deref().is_some_and(|t| t >= von && t < bis);
                drin(&e)
                    || drin(&a)
                    || (e.as_deref().is_some_and(|e| e < von)
                        && a.as_deref().is_some_and(|a| a >= bis))
            })
            .collect();
        out.sort_by(|a, b| b.geaendert.cmp(&a.geaendert).then(a.id.cmp(&b.id)));
        out.truncate(grenze.clamp(1, 200));
        Ok(out)
    }
}

/// Auszug um das erste Suchwort (gefaltet verglichen), höchstens [`AUSZUG`] Zeichen.
fn auszug(text: &str, woerter: &[String]) -> String {
    // Nur der Anfang wird nach dem Wort durchsucht; das begrenzt die Kosten bei sehr langen Seiten.
    let zeichen: Vec<char> = text.chars().take(AUSZUG_SUCHFENSTER).collect();
    let gefaltet: Vec<String> = zeichen.iter().map(|c| falte(&c.to_string())).collect();
    let mut start = 0;
    if let Some(w) = woerter.first() {
        let mut acc = String::new();
        let mut index = Vec::new();
        for (i, g) in gefaltet.iter().enumerate() {
            for _ in g.chars() {
                index.push(i);
            }
            acc.push_str(g);
        }
        if let Some(pos) = acc.find(w.as_str()) {
            let zeichen_pos = acc[..pos].chars().count();
            start = index
                .get(zeichen_pos)
                .copied()
                .unwrap_or(0)
                .saturating_sub(AUSZUG / 3);
        }
    }
    let ende = (start + AUSZUG).min(zeichen.len());
    let mut s: String = zeichen[start..ende]
        .iter()
        .collect::<String>()
        .replace('\n', " ");
    if start > 0 {
        s.insert(0, '…');
    }
    if ende < zeichen.len() || text.chars().nth(ende).is_some() {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests;
