//! Lokaler Abrufindex der Ablage (Spec §4.3, ADR-0029 E2): `ablage/index.db`, abgeleitet, löschbar, nie
//! exportiert oder synchronisiert.
//!
//! * Textstellen und Ranking (FTS5/BM25) liegen im vendorten `wai-retrieval` (Mandant `ablage`, Quelle =
//!   `dat_…`). Jede Textstelle ist eine „Seite“ des Dokuments, damit Stücke und Anker hier bestimmt
//!   werden: Stücke von höchstens [`STUECK`] Zeichen je Abschnitt mit [`UEBERLAPPUNG`] Zeichen Überlappung.
//! * Begleittabellen `ablage_dok` (Felder je Datei: Fassung, `textrev`, Ordnerkette, Klasse,
//!   `nur_geraet`, Papierkorb, Stand, Stempel) und `ablage_stueck` (Seite → Abschnitt, Anker, Pfad).
//! * Filter vor dem Ranking: das Quellrecht von `wai-retrieval` wird in derselben SQL-Abfrage geprüft.
//!   Lebende Dateien tragen den Leser [`LESER_SUCHE`], Dateien im Papierkorb keinen; sie fallen damit vor
//!   dem Ranking heraus. D3: je KI-Weg (global/projekt) und Klassenprofil ein weiterer Leser
//!   ([`leser_fuer`], [`ki_suchen`]); die Feinprüfung gegen den frischen Stand macht `ki`.
//! * [`abgleichen`] ist der einzige Schreibweg: vergleicht den Stempel je Datei mit dem Stand der Ablage
//!   und schreibt nur Abweichungen (Ablegen, Löschen, Verschieben, Papierkorb, Wiederherstellen, neue
//!   Textfassung). Zweimal hintereinander ändert nichts; [`neu_aufbauen`] löscht `index.db` und gleicht ab.
//! * D6: Abgleich und Suche nehmen eine [`AblageEinbettung`] (Standard: die wirksame des Prozesses).
//!   Ohne Vektoren bleibt alles lexikalisch wie in D2; mit Vektoren mischt `wai-retrieval` beide Listen
//!   per RRF (Kappung je Datei in beiden Listen). [`vektoren_nachziehen`] bettet nach dem Einschalten alle
//!   Stücke ein.

use super::extrakt::Textfassung;
use super::semantik::AblageEinbettung;
use super::speicher::{Ablage, IndexQuelle};
use super::{datei_id_pruefen, Fehler, Res};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use wai_retrieval::{Acl, IngestDoc, RetrievalError, Retriever};

pub const INDEX: &str = "index.db";
pub const MANDANT: &str = "ablage";
/// Leser der Ablage-Suche (Oberfläche, ohne KI).
pub const LESER_SUCHE: &str = "ablage-suche";
pub const STUECK: usize = 220;
pub const UEBERLAPPUNG: usize = 40;
const KANDIDATEN: usize = 60;
const STELLEN_JE_DATEI: usize = 3;
/// Größere Textfassungen werden nur bis hierher gelesen und gekürzt indiziert (Speicherschutz).
pub const MAX_TEXTFASSUNG: u64 = 32 * 1024 * 1024;
/// Marke im Ablageordner: ein Abgleich ist fehlgeschlagen und wird beim nächsten Öffnen wiederholt
/// (sonst bliebe Text endgültig gelöschter Dateien im Index).
pub const OFFEN: &str = "index.offen";
const CHECKPOINT_VERSUCHE: u32 = 5;

/// Serialisiert die Schreiber ([`abgleichen`], [`neu_aufbauen`]). Die Suche nimmt sie nicht: SQLite im
/// WAL-Modus liest neben einem Schreiber, und ein langer Abgleich (große Textfassungen) darf die Suche
/// nicht blockieren.
static SPERRE: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// Nur [`neu_aufbauen`] hält sie schreibend (Dateien löschen), die Suche lesend (Datenbank öffnen).
static DATEIEN_SPERRE: std::sync::RwLock<()> = std::sync::RwLock::new(());
/// Wartende Schreiber der [`DATEIEN_SPERRE`]. Die Sperre ist nicht überall fair (Windows: SRWLock);
/// das schrittweise [`vektoren_nachziehen`] nimmt deshalb keinen neuen Schritt, solange ein Schreiber
/// wartet, und lässt ihm ausdrücklich den Vortritt.
static SCHREIBER_WARTET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static SCHREIBER_FERTIG: (std::sync::Mutex<()>, std::sync::Condvar) =
    (std::sync::Mutex::new(()), std::sync::Condvar::new());
/// Begonnene Schritte von [`vektoren_nachziehen`] (Test der Übergabe).
static SCHRITTE_BEGONNEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Schreibsperre der Dateien, als wartender Schreiber angemeldet (bis zum Freigeben).
struct DateienExklusiv {
    _g: std::sync::RwLockWriteGuard<'static, ()>,
}

impl Drop for DateienExklusiv {
    fn drop(&mut self) {
        // Läuft vor dem Freigeben der Sperre (Feld): ein geweckter Leser wartet dann noch kurz an
        // der Sperre selbst.
        SCHREIBER_WARTET.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        let _m = SCHREIBER_FERTIG.0.lock().unwrap_or_else(|e| e.into_inner());
        SCHREIBER_FERTIG.1.notify_all();
    }
}

fn dateien_exklusiv() -> DateienExklusiv {
    SCHREIBER_WARTET.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    DateienExklusiv {
        _g: DATEIEN_SPERRE.write().unwrap_or_else(|e| e.into_inner()),
    }
}

/// Wartet ohne Sperre, bis kein Schreiber der Dateien mehr ansteht.
fn schreibern_vortritt_lassen() {
    use std::sync::atomic::Ordering;
    let mut m = SCHREIBER_FERTIG.0.lock().unwrap_or_else(|e| e.into_inner());
    while SCHREIBER_WARTET.load(Ordering::SeqCst) > 0 {
        m = SCHREIBER_FERTIG
            .1
            .wait_timeout(m, std::time::Duration::from_millis(50))
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
}

fn sperren() -> std::sync::MutexGuard<'static, ()> {
    SPERRE.lock().unwrap_or_else(|e| e.into_inner())
}

fn fehler() -> Fehler {
    Fehler::neu("ablage.index", "Der Suchindex ist gerade nicht verfügbar.")
}

/// Quellrecht einer Datei im Index: [`LESER_SUCHE`] für lebende Dateien, dazu (D3) je KI-Weg und
/// Klassenprofil ein Leser [`ki_leser`]. So fallen nicht freigegebene oder zu hoch eingestufte Dateien
/// schon in SQL vor dem Ranking heraus (Spec §4.3). Maßgeblich bleibt die Prüfung gegen den frischen
/// Stand in `ki::auszuege`; das Quellrecht verhindert nur, dass gesperrte Dateien Plätze belegen.
pub(crate) fn leser_fuer(q: &IndexQuelle) -> Vec<String> {
    use super::zusammenfuehren::Klasse;
    if q.papierkorb {
        return vec![];
    }
    let mut l = vec![LESER_SUCHE.to_string()];
    let Some(klasse) = Klasse::aus_text(&q.klasse) else {
        return l;
    };
    let wege: &[KiWeg] = match q.ki.as_str() {
        "an" => &[KiWeg::Global, KiWeg::Projekt],
        "aus" => &[KiWeg::Projekt],
        _ => &[],
    };
    for weg in wege {
        for p in KiProfil::ALLE {
            if p.erlaubt(klasse) {
                l.push(ki_leser(*weg, p));
            }
        }
    }
    l
}

/// KI-Weg im Quellrecht des Index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KiWeg {
    /// Assistent: nur Unterbäume mit `an`.
    Global,
    /// Projekt: Mitgliedschaftsschalter; `gesperrt` schließt aus.
    Projekt,
}

/// Klassenprofil des Ziels im Quellrecht des Index (grob; fein prüft `ki::klasse_erlaubt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KiProfil {
    /// Lokales Modell: jede bekannte Klasse.
    Lokal,
    /// Cloud bis `public`.
    CloudPublic,
    /// Cloud bis `internal`.
    CloudInternal,
    /// Cloud bis `confidential` (nur mit Ausnahme; die Ausnahme prüft `ki`).
    CloudConfidential,
}

impl KiProfil {
    pub const ALLE: [KiProfil; 4] = [
        KiProfil::Lokal,
        KiProfil::CloudPublic,
        KiProfil::CloudInternal,
        KiProfil::CloudConfidential,
    ];

    fn erlaubt(self, k: super::zusammenfuehren::Klasse) -> bool {
        use super::zusammenfuehren::Klasse;
        match self {
            KiProfil::Lokal => k != Klasse::Unbekannt,
            KiProfil::CloudPublic => k == Klasse::Public,
            KiProfil::CloudInternal => k <= Klasse::Internal,
            KiProfil::CloudConfidential => k <= Klasse::Confidential,
        }
    }

    fn code(self) -> &'static str {
        match self {
            KiProfil::Lokal => "lokal",
            KiProfil::CloudPublic => "cloud-public",
            KiProfil::CloudInternal => "cloud-internal",
            KiProfil::CloudConfidential => "cloud-confidential",
        }
    }
}

/// Leser im Quellrecht für einen KI-Weg und ein Klassenprofil.
pub fn ki_leser(weg: KiWeg, p: KiProfil) -> String {
    let w = match weg {
        KiWeg::Global => "global",
        KiWeg::Projekt => "projekt",
    };
    format!("ablage-ki-{w}-{}", p.code())
}

// Aufrufer `ki::auszuege` ist bis lva#365 P4c nur in Tests erreichbar.
#[cfg_attr(not(test), allow(dead_code))]
/// Eine Textstelle für die KI, roh aus dem Index (vor der Prüfung gegen den frischen Stand).
#[derive(Debug, Clone, PartialEq)]
pub struct KiStelle {
    pub dat: String,
    pub anker: String,
    pub pfad: Vec<String>,
    pub text: String,
    pub rang: f64,
}

// Aufrufer `ki::auszuege` ist bis lva#365 P4c nur in Tests erreichbar.
#[cfg_attr(not(test), allow(dead_code))]
/// Ergebnis einer KI-Suche: Stellen (beste zuerst) und der Indexstand je Datei (`fas`, `textrev`).
#[derive(Debug, Clone, Default)]
pub struct KiSuche {
    pub stellen: Vec<KiStelle>,
    pub indiziert: BTreeMap<String, (String, Option<String>)>,
}

// Aufrufer `ki::auszuege` ist bis lva#365 P4c nur in Tests erreichbar.
#[cfg_attr(not(test), allow(dead_code))]
/// Suche für die KI (D3): wie [`suchen_mit`], aber mit dem Leser des KI-Wegs (Filter in SQL vor dem
/// Ranking) und ohne Gruppierung; dazu der Indexstand je Datei, damit der Aufrufer veraltete Stellen
/// verwirft und die Abdeckung meldet. Kein Index: leeres Ergebnis.
pub fn ki_suchen(
    a: &Ablage,
    anfrage: &str,
    emb: &AblageEinbettung,
    leser: &str,
    kandidaten: usize,
) -> Res<KiSuche> {
    let anfrage: String = anfrage.chars().take(500).collect();
    let pfad = index_pfad(a)?;
    if !pfad.is_file() {
        return Ok(KiSuche::default());
    }
    let _d = DATEIEN_SPERRE.read().unwrap_or_else(|e| e.into_inner());
    let c = verbinden(&pfad)?;
    let mut indiziert = BTreeMap::new();
    {
        let mut q = c
            .prepare("SELECT dat, fas, textrev FROM ablage_dok")
            .map_err(|_| fehler())?;
        let zeilen = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|_| fehler())?;
        for z in zeilen {
            let (d, f, t) = z.map_err(|_| fehler())?;
            indiziert.insert(d, (f, t));
        }
    }
    if wai_retrieval::content_tokens(&anfrage).is_empty() {
        return Ok(KiSuche {
            stellen: vec![],
            indiziert,
        });
    }
    let trace = format!("{:032x}", ulid::Ulid::new().0);
    let antwort = laufzeit()?.block_on(async {
        let r = Retriever::open(&pfad, emb.clone())
            .await
            .map_err(|_| fehler())?
            .with_max_per_source(STELLEN_JE_DATEI);
        let erg = r
            .query(&principal(leser), &trace, &anfrage, kandidaten)
            .await
            .map_err(|_| fehler());
        r.close().await;
        erg
    })?;
    let mut stellen = Vec::new();
    for z in &antwort.citations {
        if datei_id_pruefen(&z.artifact_id).is_err() {
            continue;
        }
        let seite = z.locator.page.unwrap_or(0) as i64;
        let zeile: Option<(String, String)> = c
            .query_row(
                "SELECT anker, pfad FROM ablage_stueck WHERE dat = ?1 AND seite = ?2",
                params![z.artifact_id, seite],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| fehler())?;
        let Some((anker, pfad)) = zeile else {
            continue;
        };
        stellen.push(KiStelle {
            dat: z.artifact_id.clone(),
            anker,
            pfad: serde_json::from_str(&pfad).unwrap_or_default(),
            text: z.quote.clone(),
            rang: z.score,
        });
    }
    Ok(KiSuche { stellen, indiziert })
}

/// Fundstelle in einer Datei.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct AblageFundstelle {
    /// Anker nach Markenformat v1 (`seite=3`, `absatz=12`, `blatt=…&zeilen=2-21`, `teil=kopf`, …).
    pub anker: String,
    /// Überschriftenpfad (DOCX/ODT/HTML) bzw. Blattname.
    pub pfad: Vec<String>,
    pub auszug: String,
}

/// Suchtreffer: Name und/oder Inhalt einer lebenden Datei.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct AblageSuchtreffer {
    pub dat: String,
    pub name: String,
    pub ordner: String,
    pub typ: String,
    /// Bester Rang der Fundstellen (0..1).
    pub rang: f64,
    pub stellen: Vec<AblageFundstelle>,
}

/// Stücke eines Abschnitts: höchstens `max` Zeichen, Überlappung `ueber` Zeichen, Schnitt möglichst an
/// Leerraum. Arbeitet auf Byte-Positionen an Zeichengrenzen (kein `Vec<char>` des ganzen Abschnitts).
pub fn stuecke(text: &str, max: usize, ueber: usize) -> Vec<String> {
    /// Byte-Position `n` Zeichen nach `pos` (höchstens das Ende).
    fn vor(t: &str, pos: usize, n: usize) -> usize {
        t[pos..]
            .char_indices()
            .nth(n)
            .map_or(t.len(), |(i, _)| pos + i)
    }
    /// Byte-Position `n` Zeichen vor `pos` (höchstens der Anfang).
    fn zurueck(t: &str, pos: usize, n: usize) -> usize {
        if n == 0 {
            return pos;
        }
        t[..pos]
            .char_indices()
            .rev()
            .nth(n - 1)
            .map_or(0, |(i, _)| i)
    }
    let n = text.len();
    let mut aus = Vec::new();
    let mut start = 0usize;
    while start < n {
        start += text[start..].len() - text[start..].trim_start().len();
        if start >= n {
            break;
        }
        let mut ende = vor(text, start, max);
        if ende < n {
            let halb = vor(text, start, max / 2);
            if let Some((i, _)) = text[halb..ende]
                .char_indices()
                .rev()
                .find(|(_, c)| c.is_whitespace())
            {
                ende = halb + i;
            }
        }
        aus.push(text[start..ende].trim().to_string());
        if ende >= n {
            break;
        }
        let erstes = text[start..].chars().next().map_or(1, char::len_utf8);
        let mut naechster = zurueck(text, ende, ueber).max(start + erstes);
        // Am Wortanfang beginnen.
        while naechster < ende
            && !text[..naechster]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            naechster += text[naechster..].chars().next().map_or(1, char::len_utf8);
        }
        start = naechster;
    }
    aus.retain(|s| !s.is_empty());
    aus
}

fn stempel(q: &IndexQuelle) -> String {
    let mut h = Sha256::new();
    for teil in [
        q.fas.as_str(),
        q.textrev.as_deref().unwrap_or("-"),
        &q.ordnerkette.join("/"),
        &q.klasse,
        &q.ki,
        if q.nur_geraet { "1" } else { "0" },
        if q.papierkorb { "1" } else { "0" },
        &q.stand,
        // v2 (D3): Leser je KI-Weg und Klassenprofil im Quellrecht.
        "v2",
    ] {
        h.update(teil.as_bytes());
        h.update([0u8]);
    }
    super::extrakt::hex(&h.finalize())
}

fn verbinden(pfad: &Path) -> Res<Connection> {
    let c = Connection::open(pfad).map_err(|_| fehler())?;
    c.busy_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| fehler())?;
    // Gelöschte Seiten überschreiben: Text entfernter Dokumente bleibt nicht in der Datei liegen.
    c.pragma_update(None, "secure_delete", "ON")
        .map_err(|_| fehler())?;
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS ablage_dok(
            dat TEXT PRIMARY KEY, stempel TEXT NOT NULL, version INTEGER NOT NULL,
            fas TEXT NOT NULL, textrev TEXT, ordnerkette TEXT NOT NULL, klasse TEXT NOT NULL,
            nur_geraet INTEGER NOT NULL, papierkorb INTEGER NOT NULL, stand TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS ablage_stueck(
            dat TEXT NOT NULL, seite INTEGER NOT NULL, abschnitt INTEGER NOT NULL,
            anker TEXT NOT NULL, pfad TEXT NOT NULL, PRIMARY KEY(dat, seite));",
    )
    .map_err(|_| fehler())?;
    Ok(c)
}

/// Test: Code unter der Schreibersperre ausführen.
#[cfg(test)]
pub(crate) fn mit_sperre<T>(f: impl FnOnce() -> T) -> T {
    let _g = sperren();
    f()
}

/// Test: begonnene Schritte von [`vektoren_nachziehen`] (prozessweit).
#[cfg(test)]
pub(crate) fn schritte_begonnen() -> usize {
    SCHRITTE_BEGONNEN.load(std::sync::atomic::Ordering::SeqCst)
}

/// Test: meldet sich als Schreiber der Dateien an wie [`neu_aufbauen`] und liefert die begonnenen
/// Schritte des Nachziehens bei der Anmeldung und beim Erhalt der Sperre.
#[cfg(test)]
pub(crate) fn dateisperre_uebergabe() -> (usize, usize) {
    use std::sync::atomic::Ordering;
    SCHREIBER_WARTET.fetch_add(1, Ordering::SeqCst);
    let vorher = SCHRITTE_BEGONNEN.load(Ordering::SeqCst);
    let g = DateienExklusiv {
        _g: DATEIEN_SPERRE.write().unwrap_or_else(|e| e.into_inner()),
    };
    let nachher = SCHRITTE_BEGONNEN.load(Ordering::SeqCst);
    drop(g);
    (vorher, nachher)
}

/// `PRAGMA secure_delete` der eigenen Verbindung und der von `wai-retrieval` (Test).
#[cfg(test)]
pub(crate) fn secure_delete_stand(a: &Ablage) -> Res<(i64, i64)> {
    let pfad = index_pfad(a)?;
    let c = verbinden(&pfad)?;
    let eigene: i64 = c
        .query_row("PRAGMA secure_delete", [], |r| r.get(0))
        .map_err(|_| fehler())?;
    let fremde = laufzeit()?.block_on(async {
        let r = Retriever::open(&pfad, AblageEinbettung::Keine)
            .await
            .map_err(|_| fehler())?;
        r.secure_delete().await.map_err(|_| fehler())
    })?;
    Ok((eigene, fremde))
}

fn laufzeit() -> Res<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| fehler())
}

fn index_pfad(a: &Ablage) -> Res<PathBuf> {
    // `ablage/` selbst darf keine Verknüpfung sein; `index.db` auch nicht.
    if let Ok(m) = std::fs::symlink_metadata(a.dir()) {
        if super::ist_umweg(&m) {
            return Err(fehler());
        }
    }
    let p = a.dir().join(INDEX);
    if let Ok(m) = std::fs::symlink_metadata(&p) {
        if super::ist_umweg(&m) || super::pfad_hartverknuepft(&p) {
            return Err(fehler());
        }
    }
    Ok(p)
}

/// Textfassung lesen und prüfen (passende `textrev`); `None` bei fehlender oder fremder Datei.
/// Der zweite Wert ist `true`, wenn die Fassung über [`MAX_TEXTFASSUNG`] lag und nur ihr Anfang (volle
/// Abschnitte) gelesen wurde.
fn textfassung_lesen(a: &Ablage, dat: &str, textrev: &str) -> Option<(Textfassung, bool)> {
    begrenzt_lesen(&a.text_pfad(dat, textrev), textrev, MAX_TEXTFASSUNG)
}

pub(crate) fn begrenzt_lesen(p: &Path, textrev: &str, max: u64) -> Option<(Textfassung, bool)> {
    use std::io::Read;
    let m = std::fs::symlink_metadata(p).ok()?;
    if super::ist_umweg(&m) || !m.is_file() {
        return None;
    }
    // Die Größe steht vor dem Lesen fest: nie mehr als `max` Bytes in den Speicher.
    let mut buf = Vec::with_capacity(m.len().min(max) as usize);
    std::fs::File::open(p)
        .ok()?
        .take(max.saturating_add(1))
        .read_to_end(&mut buf)
        .ok()?;
    let gekuerzt = m.len() > max || buf.len() as u64 > max;
    if gekuerzt {
        buf.truncate(max as usize);
        // Letzten vollständigen Abschnitt behalten: `abschnitte` steht zuletzt, ein Abschnitt beginnt
        // immer mit `{"anker"` (ein `"` im Text wäre als `\"` maskiert, die Folge ist also eindeutig).
        let trenner = b",{\"anker\"";
        match buf.windows(trenner.len()).rposition(|w| w == trenner) {
            Some(pos) => {
                buf.truncate(pos);
                buf.extend_from_slice(b"]}");
            }
            None => {
                // Schon der erste Abschnitt ist zu groß: seinen Text vorne behalten (auf einer
                // UTF-8-Grenze und ohne halbes Escape gekürzt) statt nichts zu indizieren.
                let t = kopf_text_kuerzen(&buf)?;
                log::info!("ablage: erster Abschnitt der Textfassung gekürzt");
                buf = t;
            }
        }
    }
    let t: Textfassung = serde_json::from_slice(&buf).ok()?;
    (t.textrev == textrev).then_some((t, gekuerzt))
}

/// Schließt abgeschnittenes JSON mit genau einem (gekürzten) Abschnitt wieder ab.
fn kopf_text_kuerzen(buf: &[u8]) -> Option<Vec<u8>> {
    let start = buf.windows(14).position(|w| w == b"\"abschnitte\":[")?;
    let rest = &buf[start..];
    let text0 = rest.windows(8).position(|w| w == b"\"text\":\"")? + 8;
    let mut ende = buf.len();
    // Bis zu 12 Bytes zurück: halbes Zeichen oder halbes Escape (`\u12`) entfernen.
    for _ in 0..12 {
        if ende <= start + text0 {
            return None;
        }
        let mut v = buf[..ende].to_vec();
        v.extend_from_slice(b"\"}]}");
        if serde_json::from_slice::<Textfassung>(&v).is_ok() {
            return Some(v);
        }
        ende -= 1;
    }
    None
}

/// Bericht eines Abgleichs (Tests, Oberfläche).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageIndexBericht {
    pub geschrieben: u32,
    pub entfernt: u32,
    pub dateien: u32,
    pub stuecke: u32,
}

/// Bringt `index.db` auf den Stand der Ablage (nur Abweichungen werden geschrieben), mit der wirksamen
/// Einbettung des Prozesses ([`super::semantik::aktuell`]). Scheitert der Abgleich, bleibt die Marke
/// [`OFFEN`] liegen ([`nachholen`] wiederholt ihn beim nächsten Öffnen).
pub fn abgleichen(a: &Ablage) -> Res<AblageIndexBericht> {
    abgleichen_mit(a, &super::semantik::aktuell())
}

/// Wie [`abgleichen`] mit ausdrücklicher Einbettung. Neue Stücke bekommen sofort Vektoren, wenn der
/// Index schon im Vektorraum von `emb` liegt; sonst bleiben sie lexikalisch bis [`vektoren_nachziehen`].
pub fn abgleichen_mit(a: &Ablage, emb: &AblageEinbettung) -> Res<AblageIndexBericht> {
    let _g = sperren();
    let marke = a.dir().join(OFFEN);
    match abgleichen_gesperrt(a, emb) {
        Ok(b) => {
            let _ = std::fs::remove_file(&marke);
            Ok(b)
        }
        Err(e) => {
            if a.dir().is_dir() {
                let _ = std::fs::write(&marke, b"");
            }
            Err(e)
        }
    }
}

/// Wiederholt einen früher fehlgeschlagenen Abgleich (Marke [`OFFEN`]); sonst nichts.
pub fn nachholen(a: &Ablage) -> Res<Option<AblageIndexBericht>> {
    if std::fs::symlink_metadata(a.dir().join(OFFEN)).is_err() {
        return Ok(None);
    }
    // Läuft schon ein Abgleich (Schreiber), holt dieser selbst nach (Erfolg räumt die Marke); die Seite
    // „Ablage“ wartet deshalb nicht auf die Sperre.
    let Ok(g) = SPERRE.try_lock() else {
        return Ok(None);
    };
    drop(g);
    abgleichen(a).map(Some)
}

fn abgleichen_gesperrt(a: &Ablage, emb: &AblageEinbettung) -> Res<AblageIndexBericht> {
    let quellen = a.index_quellen()?;
    let pfad = index_pfad(a)?;
    std::fs::create_dir_all(a.dir()).map_err(|_| fehler())?;
    let mut c = verbinden(&pfad)?;
    let mut bekannt: BTreeMap<String, (String, i64)> = BTreeMap::new();
    {
        let mut q = c
            .prepare("SELECT dat, stempel, version FROM ablage_dok")
            .map_err(|_| fehler())?;
        let zeilen = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|_| fehler())?;
        for z in zeilen {
            let (d, s, v): (String, String, i64) = z.map_err(|_| fehler())?;
            bekannt.insert(d, (s, v));
        }
    }
    let soll: BTreeSet<&str> = quellen.iter().map(|q| q.dat.as_str()).collect();
    // Verwaist: Dokumente im Retriever ohne lebenden Eintrag (z. B. nach einem Abbruch zwischen beiden
    // Schreibvorgängen) werden wie gelöschte Dateien entfernt.
    let mut waisen: BTreeSet<String> = BTreeSet::new();
    if let Ok(mut q) = c.prepare("SELECT source_id FROM docs WHERE tenant = ?1") {
        if let Ok(zeilen) = q.query_map(params![MANDANT], |r| r.get::<_, String>(0)) {
            waisen.extend(zeilen.flatten().filter(|d| !soll.contains(d.as_str())));
        }
    }
    let weg: Vec<String> = bekannt
        .keys()
        .filter(|d| !soll.contains(d.as_str()))
        .cloned()
        .chain(waisen.into_iter().filter(|d| !bekannt.contains_key(d)))
        .collect();
    let neu: Vec<(&IndexQuelle, String, i64)> = quellen
        .iter()
        .filter_map(|q| {
            let s = stempel(q);
            match bekannt.get(&q.dat) {
                Some((alt, _)) if *alt == s => None,
                Some((_, v)) => Some((q, s, v + 1)),
                None => Some((q, s, 1)),
            }
        })
        .collect();
    let mut bericht = AblageIndexBericht {
        dateien: quellen.len() as u32,
        ..Default::default()
    };
    if !weg.is_empty() || !neu.is_empty() {
        let rt = laufzeit()?;
        rt.block_on(async {
            let r = Retriever::open(&pfad, emb.clone())
                .await
                .map_err(|_| fehler())?;
            let erg = schreiben(a, &r, &mut c, &weg, &neu, &mut bericht).await;
            r.close().await;
            erg
        })?;
    }
    // Stücke ohne Dokumenteintrag (verwaist) fallen weg.
    c.execute(
        "DELETE FROM ablage_stueck WHERE dat NOT IN (SELECT dat FROM ablage_dok)",
        [],
    )
    .map_err(|_| fehler())?;
    bericht.stuecke = c
        .query_row("SELECT COUNT(*) FROM ablage_stueck", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|_| fehler())? as u32;
    // WAL zurückschreiben und kürzen: gelöschter Text bleibt nicht in `index.db-wal` liegen.
    checkpoint(&c)?;
    Ok(bericht)
}

/// `wal_checkpoint(TRUNCATE)`: Spalte 0 ist `busy` (≠ 0: ein Leser hielt den Abschluss auf). Kurz
/// wiederholen; bleibt es busy, ist der Abgleich nicht abgeschlossen (Marke [`OFFEN`] über den Aufrufer,
/// [`nachholen`] wiederholt später).
pub(crate) fn checkpoint(c: &Connection) -> Res<()> {
    for versuch in 0..CHECKPOINT_VERSUCHE {
        let busy: i64 = c
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
            .map_err(|_| fehler())?;
        if busy == 0 {
            return Ok(());
        }
        if versuch + 1 < CHECKPOINT_VERSUCHE {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
    }
    Err(fehler())
}

async fn schreiben(
    a: &Ablage,
    r: &Retriever<AblageEinbettung>,
    c: &mut Connection,
    weg: &[String],
    neu: &[(&IndexQuelle, String, i64)],
    bericht: &mut AblageIndexBericht,
) -> Res<()> {
    {
        {
            for d in weg {
                r.delete(d).await.map_err(|_| fehler())?;
                let tx = c.transaction().map_err(|_| fehler())?;
                tx.execute("DELETE FROM ablage_stueck WHERE dat = ?1", params![d])
                    .map_err(|_| fehler())?;
                tx.execute("DELETE FROM ablage_dok WHERE dat = ?1", params![d])
                    .map_err(|_| fehler())?;
                tx.commit().map_err(|_| fehler())?;
                bericht.entfernt += 1;
            }
            for (q, s, version) in neu {
                let text = q
                    .textrev
                    .as_deref()
                    .and_then(|rev| textfassung_lesen(a, &q.dat, rev))
                    .map(|(t, gekuerzt)| {
                        if gekuerzt {
                            log::info!("ablage: Textfassung gekürzt indiziert ({})", q.dat);
                        }
                        t
                    });
                let mut seiten: Vec<String> = Vec::new();
                let mut zuordnung: Vec<(usize, String, String)> = Vec::new();
                if let Some(t) = &text {
                    for (i, ab) in t.abschnitte.iter().enumerate() {
                        let pfad = serde_json::to_string(&ab.pfad).unwrap_or_else(|_| "[]".into());
                        for st in stuecke(&ab.text, STUECK, UEBERLAPPUNG) {
                            seiten.push(st);
                            zuordnung.push((i, ab.anker.clone(), pfad.clone()));
                        }
                    }
                }
                if seiten.is_empty() {
                    r.delete(&q.dat).await.map_err(|_| fehler())?;
                } else {
                    let leser = leser_fuer(q);
                    r.ingest(&IngestDoc {
                        source_id: q.dat.clone(),
                        tenant: MANDANT.into(),
                        version: (*version).clamp(1, u32::MAX as i64) as u32,
                        pages: seiten.clone(),
                        acl: Acl::Principals(leser),
                    })
                    .await
                    .map_err(|_| fehler())?;
                }
                let tx = c.transaction().map_err(|_| fehler())?;
                tx.execute("DELETE FROM ablage_stueck WHERE dat = ?1", params![q.dat])
                    .map_err(|_| fehler())?;
                for (seite, (abschnitt, anker, pfad)) in zuordnung.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO ablage_stueck(dat, seite, abschnitt, anker, pfad) VALUES(?1,?2,?3,?4,?5)",
                        params![q.dat, seite as i64 + 1, *abschnitt as i64, anker, pfad],
                    )
                    .map_err(|_| fehler())?;
                }
                tx.execute(
                    "INSERT OR REPLACE INTO ablage_dok(dat, stempel, version, fas, textrev, ordnerkette,
                        klasse, nur_geraet, papierkorb, stand) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![
                        q.dat,
                        s,
                        version,
                        q.fas,
                        q.textrev,
                        q.ordnerkette.join("/"),
                        q.klasse,
                        q.nur_geraet,
                        q.papierkorb,
                        q.stand
                    ],
                )
                .map_err(|_| fehler())?;
                tx.commit().map_err(|_| fehler())?;
                bericht.geschrieben += 1;
            }
        }
    }
    Ok(())
}

/// „Index neu aufbauen“: `index.db` löschen und vollständig abgleichen (Extraktor-/Embedder-Wechsel,
/// beschädigter Index).
pub fn neu_aufbauen(a: &Ablage) -> Res<AblageIndexBericht> {
    neu_aufbauen_mit(a, &super::semantik::aktuell())
}

/// Wie [`neu_aufbauen`] mit ausdrücklicher Einbettung.
pub fn neu_aufbauen_mit(a: &Ablage, emb: &AblageEinbettung) -> Res<AblageIndexBericht> {
    {
        let _g = sperren();
        let _d = dateien_exklusiv();
        let p = index_pfad(a)?;
        for endung in ["", "-wal", "-shm", "-journal"] {
            let q = PathBuf::from(format!("{}{endung}", p.display()));
            match std::fs::remove_file(&q) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(fehler()),
            }
        }
    }
    abgleichen_mit(a, emb)
}

/// Stücke je Schritt in [`vektoren_nachziehen`] (e5-small: rund 0,3 s auf einem Laptop-Kern).
const NACHZIEHEN_STAPEL: usize = 32;
/// Konflikte (paralleler Abgleich, Neuaufbau), nach denen [`vektoren_nachziehen`] aufgibt.
const NACHZIEHEN_KONFLIKTE: u32 = 5;

/// Bettet Stücke ohne Vektor ein, schrittweise zu [`NACHZIEHEN_STAPEL`] (Einschalten, Modellwechsel,
/// Stücke aus der lexikalischen Zeit). Liegt der Index noch in einem anderen Vektorraum, wird er
/// zuerst ohne Einbetten übernommen (alte Vektoren weg). Jeder Schritt öffnet den Index nur kurz unter
/// der Lesesperre der Dateien und schreibt in einer kurzen Transaktion: [`neu_aufbauen`] und
/// [`abgleichen`] warten höchstens einen Schritt. Konflikte mit parallelen Schreibern werden begrenzt
/// wiederholt. Liefert die Zahl eingebetteter Stücke.
pub fn vektoren_nachziehen(a: &Ablage, emb: &AblageEinbettung) -> Res<u32> {
    if !emb.ist_semantisch() {
        return Ok(0);
    }
    let pfad = index_pfad(a)?;
    let rt = laufzeit()?;
    let mut gesamt: u32 = 0;
    let mut konflikte = 0u32;
    // Obergrenze der Schritte: die anfangs fehlenden Stücke plus Spielraum für neu hinzukommende.
    let mut runden: Option<u64> = None;
    loop {
        let schritt: Res<Option<usize>> = {
            // Ausdrückliche Übergabe: ein wartender Schreiber (Neuaufbau) kommt vor dem nächsten
            // Schritt dran, auch wenn die Sperre selbst Leser bevorzugt.
            let _d = loop {
                schreibern_vortritt_lassen();
                let d = DATEIEN_SPERRE.read().unwrap_or_else(|e| e.into_inner());
                SCHRITTE_BEGONNEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if SCHREIBER_WARTET.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                    break d;
                }
                SCHRITTE_BEGONNEN.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                drop(d);
            };
            if !pfad.is_file() {
                return Ok(gesamt);
            }
            rt.block_on(async {
                let r = Retriever::open(&pfad, emb.clone())
                    .await
                    .map_err(|_| fehler())?;
                let erg = async {
                    r.adopt_space(MANDANT).await?;
                    if runden.is_none() {
                        let st = r.status_for(MANDANT).await?;
                        let fehlen = st.missing_vectors.max(0) as u64;
                        runden = Some(fehlen / NACHZIEHEN_STAPEL as u64 * 2 + 8);
                    }
                    r.fill_missing(MANDANT, NACHZIEHEN_STAPEL).await
                }
                .await;
                r.close().await;
                match erg {
                    Ok(n) => Ok(Some(n)),
                    Err(RetrievalError::Conflict(_)) => Ok(None),
                    Err(_) => Err(fehler()),
                }
            })
        };
        match schritt? {
            Some(0) => return Ok(gesamt),
            Some(n) => gesamt = gesamt.saturating_add(n.min(u32::MAX as usize) as u32),
            None => {
                konflikte += 1;
                if konflikte >= NACHZIEHEN_KONFLIKTE {
                    return Err(fehler());
                }
            }
        }
        match runden.as_mut() {
            Some(0) => return Err(fehler()),
            Some(r) => *r -= 1,
            None => {}
        }
    }
}

/// Zustand der Vektoren für die Oberfläche: (Neuaufbau nötig, Stücke ohne Vektor). Ohne Index: (false, 0).
pub fn vektor_stand(a: &Ablage, emb: &AblageEinbettung) -> Res<(bool, u32)> {
    if !emb.ist_semantisch() {
        return Ok((false, 0));
    }
    let pfad = index_pfad(a)?;
    let _d = DATEIEN_SPERRE.read().unwrap_or_else(|e| e.into_inner());
    if !pfad.is_file() {
        return Ok((false, 0));
    }
    laufzeit()?.block_on(async {
        let r = Retriever::open(&pfad, emb.clone())
            .await
            .map_err(|_| fehler())?;
        let st = r.status_for(MANDANT).await.map_err(|_| fehler());
        r.close().await;
        let st = st?;
        Ok((
            st.reindex_needed,
            st.missing_vectors.clamp(0, u32::MAX as i64) as u32,
        ))
    })
}

fn leser() -> wai_security::Principal {
    principal(LESER_SUCHE)
}

fn principal(akteur: &str) -> wai_security::Principal {
    use wai_security::{
        resolve_principal, ActorId, ClientClaims, DataClass, Role, ServerSession, TenantId,
    };
    let sitzung = ServerSession {
        tenant: TenantId(MANDANT.into()),
        actor: ActorId(akteur.into()),
        role: Role::Viewer,
        max_class: DataClass::Restricted,
        tool_rights: Default::default(),
    };
    resolve_principal(&sitzung, &ClientClaims::default()).expect("fester, gültiger Mandant")
}

/// Volltextsuche (ohne KI): Fundstellen lebender Dateien mit Auszug und Anker, beste zuerst. Der
/// Papierkorb ist schon in SQL ausgeschlossen; zur Sicherheit wird gegen den aktuellen Stand gefiltert.
pub fn suchen(a: &Ablage, anfrage: &str) -> Res<Vec<AblageSuchtreffer>> {
    suchen_mit(a, anfrage, &super::semantik::aktuell())
}

/// Wie [`suchen`] mit ausdrücklicher Einbettung: mit Vektoren hybrid (BM25 + Kosinus, RRF), sonst
/// lexikalisch. Die Vektorliste wirkt nur, wenn der Index im Vektorraum von `emb` liegt.
pub fn suchen_mit(
    a: &Ablage,
    anfrage: &str,
    emb: &AblageEinbettung,
) -> Res<Vec<AblageSuchtreffer>> {
    let anfrage: String = anfrage.chars().take(200).collect();
    if wai_retrieval::content_tokens(&anfrage).is_empty() {
        return Ok(vec![]);
    }
    let pfad = index_pfad(a)?;
    if !pfad.is_file() {
        return Ok(vec![]);
    }
    // Nur gegen das Löschen der Dateien sperren (`neu_aufbauen`); neben einem laufenden Abgleich liest die
    // Suche weiter (WAL).
    let _d = DATEIEN_SPERRE.read().unwrap_or_else(|e| e.into_inner());
    let trace = format!("{:032x}", ulid::Ulid::new().0);
    let antwort = laufzeit()?.block_on(async {
        let r = Retriever::open(&pfad, emb.clone())
            .await
            .map_err(|_| fehler())?
            // Ein Dokument mit vielen Treffern soll nicht alle Kandidatenplätze belegen.
            .with_max_per_source(STELLEN_JE_DATEI);
        let erg = r
            .query(&leser(), &trace, &anfrage, KANDIDATEN)
            .await
            .map_err(|_| fehler());
        r.close().await;
        erg
    })?;
    let c = verbinden(&pfad)?;
    let dats: Vec<String> = antwort
        .citations
        .iter()
        .map(|z| z.artifact_id.clone())
        .filter(|d| datei_id_pruefen(d).is_ok())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let infos = a.treffer_infos(&dats)?;
    let mut je_datei: BTreeMap<String, AblageSuchtreffer> = BTreeMap::new();
    let mut reihenfolge: Vec<String> = Vec::new();
    for z in &antwort.citations {
        let Some(info) = infos.get(&z.artifact_id) else {
            continue;
        };
        let seite = z.locator.page.unwrap_or(0) as i64;
        let zeile: Option<(String, String, bool)> = c
            .query_row(
                "SELECT s.anker, s.pfad, d.papierkorb FROM ablage_stueck s
                 JOIN ablage_dok d ON d.dat = s.dat WHERE s.dat = ?1 AND s.seite = ?2",
                params![z.artifact_id, seite],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| fehler())?;
        let Some((anker, pfad, im_korb)) = zeile else {
            continue;
        };
        if im_korb {
            continue;
        }
        let t = je_datei.entry(z.artifact_id.clone()).or_insert_with(|| {
            reihenfolge.push(z.artifact_id.clone());
            AblageSuchtreffer {
                dat: z.artifact_id.clone(),
                name: info.name.clone(),
                ordner: info.ordner.clone(),
                typ: info.typ.clone(),
                rang: z.score,
                stellen: vec![],
            }
        });
        if t.stellen.len() >= STELLEN_JE_DATEI || t.stellen.iter().any(|s| s.anker == anker) {
            continue;
        }
        t.stellen.push(AblageFundstelle {
            anker,
            pfad: serde_json::from_str(&pfad).unwrap_or_default(),
            auszug: z.quote.clone(),
        });
    }
    Ok(reihenfolge
        .into_iter()
        .filter_map(|d| je_datei.remove(&d))
        .collect())
}
