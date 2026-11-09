//! Lernspeicher der Spracherkennung: `speech_learning.db` im App-Datenordner.
//!
//! Gespeichert werden nur kurze Paare „gehört → korrigiert“ (höchstens 3 Wörter und 64
//! Zeichen je Seite, siehe `diff`), nie ganze Sätze und nie Audio. Ein Paar wird beim
//! ersten Auftreten nur „beobachtet“, ab dem zweiten (oder durch eine Einrichtung per
//! Vorlesen) ein „vorschlag“. Übernommene Paare landen im eigenen Wortschatz
//! (`custom_words`): der Speicher berechnet nur, was dort hinzukommt oder wegfällt
//! (`WordsDelta`), und schreibt die Einstellungen nicht selbst. So bleibt er ohne Tauri
//! prüfbar. Inhalte stehen weder im Log noch in Fehlermeldungen.

use anyhow::{anyhow, bail, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use super::diff::{
    analyze, check_bereich, check_text, has_forbidden_chars, has_function_word, normalize_key,
    Mode, Pair, Verdict,
};
use super::setup::setup_text;

pub const STATUS_OBSERVED: &str = "beobachtet";
pub const STATUS_SUGGESTED: &str = "vorschlag";
pub const STATUS_ADOPTED: &str = "uebernommen";
pub const STATUS_REJECTED: &str = "abgelehnt";

/// Ab so vielen Vorkommen wird ein Paar zum Vorschlag.
pub const MIN_COUNT: u32 = 2;
/// Höchstens so viele Paare je Bereich.
pub const MAX_PAIRS: u32 = 500;
/// Längste Eintragung im eigenen Wortschatz (wie `CustomWords.tsx`).
pub const MAX_WORD_CHARS: usize = 50;
/// Höchstens so viele Paare legt ein Vorlesen an.
pub const MAX_SETUP_PAIRS: usize = 12;

const TS_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

static MIGRATIONS: &[M] = &[M::up(
    "CREATE TABLE IF NOT EXISTS learned_pairs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        bereich TEXT NOT NULL,
        gehoert TEXT NOT NULL,
        korrigiert TEXT NOT NULL,
        gehoert_key TEXT NOT NULL,
        korrigiert_key TEXT NOT NULL,
        anzahl INTEGER NOT NULL,
        zuletzt TEXT NOT NULL,
        status TEXT NOT NULL,
        entered_word TEXT,
        UNIQUE (bereich, gehoert_key, korrigiert_key)
    );
    CREATE INDEX IF NOT EXISTS learned_pairs_bereich
        ON learned_pairs(bereich, zuletzt);",
)];

/// Ein gelerntes Paar, wie es die Oberfläche sieht.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct LearnedPair {
    pub id: u32,
    pub bereich: String,
    pub gehoert: String,
    pub korrigiert: String,
    pub anzahl: u32,
    pub zuletzt: String,
    /// `beobachtet`, `vorschlag`, `uebernommen` oder `abgelehnt`.
    pub status: String,
}

/// Was `feedback` gemacht hat. `reason`: `ok`, `nothing`, `rewrite` oder `off`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct FeedbackOutcome {
    pub reason: String,
    /// Paare, die gespeichert oder hochgezählt wurden.
    pub learned: u32,
    /// Paare, die dadurch neu zum Vorschlag wurden.
    pub suggestions: u32,
    /// Paare, die dadurch (automatisch) übernommen wurden.
    pub adopted: u32,
}

impl FeedbackOutcome {
    pub fn empty(reason: &str) -> Self {
        Self {
            reason: reason.to_string(),
            learned: 0,
            suggestions: 0,
            adopted: 0,
        }
    }
}

/// Ergebnis der Einrichtung durch Vorlesen. `reason`: `ok`, `nothing` oder `rewrite`
/// (der Nutzer hat etwas ganz anderes gelesen oder die Aufnahme war unbrauchbar).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct SetupOutcome {
    pub reason: String,
    /// Anteil der Wörter, die richtig erkannt wurden (0 bis 100).
    pub matched_percent: u32,
    pub pairs: Vec<LearnedPair>,
}

/// Änderung am eigenen Wortschatz, die ein Aufruf nach sich zieht.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WordsDelta {
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

fn same_word(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn contains_word(words: &[String], word: &str) -> bool {
    words.iter().any(|w| same_word(w, word))
}

impl WordsDelta {
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }

    /// Wendet die Änderung auf die Wortliste an; `true`, wenn sich etwas geändert hat.
    pub fn apply(&self, words: &mut Vec<String>) -> bool {
        let before = words.clone();
        // Entfernt wird nur, was in genau dieser Schreibweise dasteht.
        words.retain(|w| !self.remove.iter().any(|r| r == w));
        for w in &self.add {
            if !contains_word(words, w) {
                words.push(w.clone());
            }
        }
        *words != before
    }
}

/// Die Form, in der ein Begriff in den eigenen Wortschatz geht; `None`, wenn er dort nicht passt.
pub fn custom_word(korrigiert: &str) -> Option<String> {
    let w = korrigiert.trim();
    if w.is_empty()
        || w.chars().count() > MAX_WORD_CHARS
        || has_forbidden_chars(w)
        || has_function_word(w)
    {
        return None;
    }
    Some(w.to_string())
}

fn now_ts() -> String {
    Utc::now().format(TS_FORMAT).to_string()
}

const COLUMNS: &str = "id, bereich, gehoert, korrigiert, anzahl, zuletzt, status";

fn row_to_pair(row: &rusqlite::Row<'_>) -> rusqlite::Result<LearnedPair> {
    Ok(LearnedPair {
        id: row.get::<_, i64>(0)?.max(0) as u32,
        bereich: row.get(1)?,
        gehoert: row.get(2)?,
        korrigiert: row.get(3)?,
        anzahl: row.get::<_, i64>(4)?.max(0) as u32,
        zuletzt: row.get(5)?,
        status: row.get(6)?,
    })
}

fn fetch(tx: &Transaction<'_>, id: u32) -> Result<Option<LearnedPair>> {
    Ok(tx
        .query_row(
            &format!("SELECT {COLUMNS} FROM learned_pairs WHERE id = ?1"),
            params![id],
            row_to_pair,
        )
        .optional()?)
}

fn require(tx: &Transaction<'_>, id: u32) -> Result<LearnedPair> {
    fetch(tx, id)?.ok_or_else(|| anyhow!("Den Eintrag gibt es nicht (mehr)."))
}

/// Zustand eines Laufs über mehrere Paare: der Wortschatz (um die Einträge dieses Laufs
/// ergänzt), die daraus folgende Änderung und ob Paare wegen der Obergrenze gelöscht wurden.
struct Run {
    words: Vec<String>,
    delta: WordsDelta,
    dropped: bool,
}

impl Run {
    fn new(words: &[String]) -> Self {
        Self {
            words: words.to_vec(),
            delta: WordsDelta::default(),
            dropped: false,
        }
    }
}

/// Was ein Paar nach dem Eintragen ist, und was sich dabei geändert hat.
struct Touched {
    pair: LearnedPair,
    became_suggestion: bool,
    became_adopted: bool,
}

pub struct SpeechLearningStore {
    conn: Mutex<Connection>,
}

impl SpeechLearningStore {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        // Gelöschtes wird überschrieben, nicht nur ausgetragen („Löschen ist echtes Löschen“).
        conn.pragma_update(None, "secure_delete", "ON")?;
        let migrations = Migrations::new(MIGRATIONS.to_vec());
        #[cfg(debug_assertions)]
        migrations
            .validate()
            .expect("Invalid speech learning migrations");
        migrations.to_latest(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Schreibt das Journal in die Datei und kürzt es, damit gelöschte Inhalte nirgends bleiben.
    fn checkpoint(conn: &Connection) {
        let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    }

    /// Gleicht `entered_word` mit dem Wortschatz ab: Steht ein Begriff nicht mehr (in genau dieser
    /// Schreibweise) darin, hat ihn der Nutzer entfernt und er gehört uns nicht mehr. So entfernt
    /// eine spätere Rücknahme nie ein vom Nutzer neu eingetragenes Wort.
    fn reconcile(tx: &Transaction<'_>, words: &[String]) -> Result<()> {
        let rows: Vec<(i64, String)> = {
            let mut stmt = tx.prepare(
                "SELECT id, entered_word FROM learned_pairs WHERE entered_word IS NOT NULL",
            )?;
            let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, word) in rows {
            if !words.iter().any(|w| *w == word) {
                tx.execute(
                    "UPDATE learned_pairs SET entered_word = NULL WHERE id = ?1",
                    params![id],
                )?;
            }
        }
        Ok(())
    }

    /// Nimmt ein Paar auf oder zählt es hoch. `floor` ist die Mindestzahl, auf die das Paar
    /// gesetzt wird (1 bei einer Korrektur, `MIN_COUNT` bei der Einrichtung per Vorlesen).
    /// `words` ist der Wortschatz, gegen den eine Übernahme geprüft wird; Einträge dieses
    /// Laufs werden angehängt, damit zwei Paare mit demselben Begriff ihn nur einmal eintragen.
    /// `None`, wenn der Bereich voll ist und nichts mehr weichen kann.
    fn touch(
        tx: &Transaction<'_>,
        bereich: &str,
        pair: &Pair,
        floor: u32,
        auto_apply: bool,
        run: &mut Run,
    ) -> Result<Option<Touched>> {
        let heard_key = normalize_key(&pair.heard);
        let corrected_key = normalize_key(&pair.corrected);
        let now = now_ts();
        let existing: Option<(u32, u32, String)> = tx
            .query_row(
                "SELECT id, anzahl, status FROM learned_pairs
                 WHERE bereich = ?1 AND gehoert_key = ?2 AND korrigiert_key = ?3",
                params![bereich, heard_key, corrected_key],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?.max(0) as u32,
                        r.get::<_, i64>(1)?.max(0) as u32,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        let (id, was_suggested) = match existing {
            Some((id, anzahl, old_status)) => {
                let next = (anzahl + 1).max(floor);
                let kept = old_status == STATUS_ADOPTED || old_status == STATUS_REJECTED;
                let status = if kept {
                    old_status.clone()
                } else if next >= MIN_COUNT {
                    STATUS_SUGGESTED.to_string()
                } else {
                    STATUS_OBSERVED.to_string()
                };
                tx.execute(
                    "UPDATE learned_pairs
                     SET gehoert = ?1, korrigiert = ?2, anzahl = ?3, zuletzt = ?4, status = ?5
                     WHERE id = ?6",
                    params![pair.heard, pair.corrected, next, now, status, id],
                )?;
                (id, old_status == STATUS_SUGGESTED)
            }
            None => {
                match make_room(tx, bereich)? {
                    None => return Ok(None),
                    Some(dropped) => run.dropped |= dropped,
                }
                let next = floor.max(1);
                let status = if next >= MIN_COUNT {
                    STATUS_SUGGESTED
                } else {
                    STATUS_OBSERVED
                };
                tx.execute(
                    "INSERT INTO learned_pairs
                     (bereich, gehoert, korrigiert, gehoert_key, korrigiert_key, anzahl, zuletzt, status)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        bereich,
                        pair.heard,
                        pair.corrected,
                        heard_key,
                        corrected_key,
                        next,
                        now,
                        status
                    ],
                )?;
                (tx.last_insert_rowid().max(0) as u32, false)
            }
        };
        let mut row = require(tx, id)?;
        let became_suggestion = row.status == STATUS_SUGGESTED && !was_suggested;
        let mut became_adopted = false;
        if auto_apply
            && row.status == STATUS_SUGGESTED
            && adopt(tx, &mut row, &mut run.words, &mut run.delta)?
        {
            became_adopted = true;
        }
        Ok(Some(Touched {
            pair: row,
            became_suggestion,
            became_adopted,
        }))
    }

    /// Lernt aus einer Korrektur. `auto_apply`: Paare mit genug Vorkommen gleich übernehmen.
    pub fn feedback(
        &self,
        bereich: &str,
        erkannt: &str,
        korrigiert: &str,
        auto_apply: bool,
        words: &[String],
    ) -> Result<(FeedbackOutcome, WordsDelta)> {
        check_bereich(bereich).map_err(|e| anyhow!(e))?;
        check_text(erkannt).map_err(|e| anyhow!(e))?;
        check_text(korrigiert).map_err(|e| anyhow!(e))?;
        let analysis = analyze(erkannt, korrigiert, Mode::Correction);
        match analysis.verdict {
            Verdict::Nothing => {
                return Ok((FeedbackOutcome::empty("nothing"), WordsDelta::default()))
            }
            Verdict::Rewrite => {
                return Ok((FeedbackOutcome::empty("rewrite"), WordsDelta::default()))
            }
            Verdict::Pairs => {}
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        Self::reconcile(&tx, words)?;
        let mut run = Run::new(words);
        let mut outcome = FeedbackOutcome::empty("ok");
        for pair in &analysis.pairs {
            let Some(t) = Self::touch(&tx, bereich, pair, 1, auto_apply, &mut run)? else {
                continue;
            };
            outcome.learned += 1;
            outcome.suggestions += u32::from(t.became_suggestion);
            outcome.adopted += u32::from(t.became_adopted);
        }
        tx.commit()?;
        if run.dropped {
            Self::checkpoint(&conn);
        }
        Ok((outcome, run.delta))
    }

    /// Wertet das Vorlesen aus: `reference` ist der Vorlesetext, `transcript` die Erkennung.
    /// Die Paare werden sofort zu Vorschlägen (der Nutzer hat gelesen, was dort steht).
    pub fn setup_evaluate(
        &self,
        bereich: &str,
        reference: &str,
        transcript: &str,
        words: &[String],
    ) -> Result<SetupOutcome> {
        check_bereich(bereich).map_err(|e| anyhow!(e))?;
        check_text(reference).map_err(|e| anyhow!(e))?;
        check_text(transcript).map_err(|e| anyhow!(e))?;
        if reference != setup_text(words).text {
            bail!("Der Vorlesetext stimmt nicht (mehr). Bitte starte die Einrichtung neu.");
        }
        let analysis = analyze(transcript, reference, Mode::Reading);
        let percent = if analysis.total == 0 {
            0
        } else {
            (analysis.matched * 100 / analysis.total) as u32
        };
        let reason = match analysis.verdict {
            Verdict::Pairs => "ok",
            Verdict::Nothing => "nothing",
            Verdict::Rewrite => "rewrite",
        };
        let mut out = SetupOutcome {
            reason: reason.to_string(),
            matched_percent: percent,
            pairs: Vec::new(),
        };
        if analysis.verdict != Verdict::Pairs {
            return Ok(out);
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        Self::reconcile(&tx, words)?;
        let mut run = Run::new(&[]);
        for pair in analysis.pairs.iter().take(MAX_SETUP_PAIRS) {
            if let Some(t) = Self::touch(&tx, bereich, pair, MIN_COUNT, false, &mut run)? {
                out.pairs.push(t.pair);
            }
        }
        tx.commit()?;
        if run.dropped {
            Self::checkpoint(&conn);
        }
        Ok(out)
    }

    /// Alle Paare eines Bereichs, jüngste zuerst.
    pub fn list(&self, bereich: &str, words: &[String]) -> Result<Vec<LearnedPair>> {
        check_bereich(bereich).map_err(|e| anyhow!(e))?;
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        Self::reconcile(&tx, words)?;
        tx.commit()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM learned_pairs WHERE bereich = ?1 ORDER BY zuletzt DESC, id DESC"
        ))?;
        let rows = stmt.query_map(params![bereich], row_to_pair)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Setzt den Status eines Paares (`vorschlag`, `uebernommen` oder `abgelehnt`).
    pub fn set_status(
        &self,
        id: u32,
        status: &str,
        words: &[String],
    ) -> Result<(LearnedPair, WordsDelta)> {
        if ![STATUS_SUGGESTED, STATUS_ADOPTED, STATUS_REJECTED].contains(&status) {
            bail!("Der Status ist ungültig.");
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        Self::reconcile(&tx, words)?;
        let mut row = require(&tx, id)?;
        let mut live = words.to_vec();
        let mut delta = WordsDelta::default();
        if status == STATUS_ADOPTED {
            if row.status != STATUS_ADOPTED && !adopt(&tx, &mut row, &mut live, &mut delta)? {
                bail!("Dieser Begriff passt nicht in den eigenen Wortschatz.");
            }
        } else if row.status != status {
            release(&tx, &row, &mut delta)?;
            tx.execute(
                "UPDATE learned_pairs SET status = ?1, entered_word = NULL WHERE id = ?2",
                params![status, id],
            )?;
            row.status = status.to_string();
        }
        tx.commit()?;
        Ok((row, delta))
    }

    /// Löscht ein Paar wirklich. Hat es einen Begriff in den Wortschatz gebracht, geht der wieder.
    pub fn delete(&self, id: u32, words: &[String]) -> Result<WordsDelta> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        Self::reconcile(&tx, words)?;
        let row = require(&tx, id)?;
        let mut delta = WordsDelta::default();
        release(&tx, &row, &mut delta)?;
        tx.execute("DELETE FROM learned_pairs WHERE id = ?1", params![id])?;
        tx.commit()?;
        Self::checkpoint(&conn);
        Ok(delta)
    }

    /// Löscht alle Paare eines Bereichs wirklich; Rückgabe: wie viele es waren.
    pub fn clear(&self, bereich: &str, words: &[String]) -> Result<(u32, WordsDelta)> {
        check_bereich(bereich).map_err(|e| anyhow!(e))?;
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        Self::reconcile(&tx, words)?;
        let rows: Vec<LearnedPair> = {
            let mut stmt = tx.prepare(&format!(
                "SELECT {COLUMNS} FROM learned_pairs WHERE bereich = ?1"
            ))?;
            let mapped = stmt.query_map(params![bereich], row_to_pair)?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut delta = WordsDelta::default();
        let gone: Vec<u32> = rows.iter().map(|r| r.id).collect();
        for row in &rows {
            release_except(&tx, row, &gone, &mut delta)?;
        }
        tx.execute(
            "DELETE FROM learned_pairs WHERE bereich = ?1",
            params![bereich],
        )?;
        tx.commit()?;
        // VACUUM schreibt die Datei neu, der Checkpoint leert das Journal.
        conn.execute_batch("VACUUM")?;
        Self::checkpoint(&conn);
        Ok((rows.len() as u32, delta))
    }
}

/// Schafft Platz für ein neues Paar: bei vollem Bereich fällt das älteste Beobachtete oder
/// Vorgeschlagene weg (`Some(true)`). `None`, wenn nichts mehr weichen darf (alles übernommen
/// oder abgelehnt).
fn make_room(tx: &Transaction<'_>, bereich: &str) -> Result<Option<bool>> {
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM learned_pairs WHERE bereich = ?1",
        params![bereich],
        |r| r.get(0),
    )?;
    if count < i64::from(MAX_PAIRS) {
        return Ok(Some(false));
    }
    let oldest: Option<i64> = tx
        .query_row(
            "SELECT id FROM learned_pairs WHERE bereich = ?1 AND status IN (?2, ?3)
             ORDER BY zuletzt ASC, id ASC LIMIT 1",
            params![bereich, STATUS_OBSERVED, STATUS_SUGGESTED],
            |r| r.get(0),
        )
        .optional()?;
    match oldest {
        Some(id) => {
            tx.execute("DELETE FROM learned_pairs WHERE id = ?1", params![id])?;
            Ok(Some(true))
        }
        None => Ok(None),
    }
}

/// Übernimmt ein Paar: Status `uebernommen`, Begriff in den Wortschatz, falls dort noch nicht.
/// Merkt sich, ob der Begriff von hier stammt (`entered_word`), damit ihn Löschen wieder
/// entfernt, ohne einen selbst eingetragenen anzutasten. `false`, wenn der Begriff nicht passt.
fn adopt(
    tx: &Transaction<'_>,
    row: &mut LearnedPair,
    words: &mut Vec<String>,
    delta: &mut WordsDelta,
) -> Result<bool> {
    let Some(word) = custom_word(&row.korrigiert) else {
        return Ok(false);
    };
    // Steht der Begriff schon im Wortschatz und hält ihn kein anderes Paar, hat der Nutzer ihn
    // selbst eingetragen: dann gehört er nicht uns und bleibt beim Löschen.
    let known = contains_word(words, &word);
    let entered = if known && !held_by_other(tx, &word, &[row.id])? {
        None
    } else {
        if !known {
            words.push(word.clone());
            delta.remove.retain(|r| !same_word(r, &word));
            delta.add.push(word.clone());
        }
        Some(word)
    };
    tx.execute(
        "UPDATE learned_pairs SET status = ?1, entered_word = ?2 WHERE id = ?3",
        params![STATUS_ADOPTED, entered, row.id],
    )?;
    row.status = STATUS_ADOPTED.to_string();
    Ok(true)
}

/// Nimmt zurück, was ein Paar in den Wortschatz gebracht hat, sofern kein anderes Paar
/// denselben Begriff dort hält.
fn release(tx: &Transaction<'_>, row: &LearnedPair, delta: &mut WordsDelta) -> Result<()> {
    release_except(tx, row, &[row.id], delta)
}

fn release_except(
    tx: &Transaction<'_>,
    row: &LearnedPair,
    ignore: &[u32],
    delta: &mut WordsDelta,
) -> Result<()> {
    let entered: Option<String> = tx
        .query_row(
            "SELECT entered_word FROM learned_pairs WHERE id = ?1",
            params![row.id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let Some(word) = entered else {
        return Ok(());
    };
    if held_by_other(tx, &word, ignore)? {
        return Ok(());
    }
    if !delta.remove.iter().any(|r| same_word(r, &word)) {
        delta.remove.push(word);
    }
    Ok(())
}

/// Hält ein Paar außerhalb von `ignore` den Begriff im Wortschatz (`entered_word`)?
fn held_by_other(tx: &Transaction<'_>, word: &str, ignore: &[u32]) -> Result<bool> {
    let mut stmt =
        tx.prepare("SELECT id, entered_word FROM learned_pairs WHERE entered_word IS NOT NULL")?;
    let others = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for other in others {
        let (id, other_word) = other?;
        if !ignore.contains(&(id.max(0) as u32)) && same_word(&other_word, word) {
            return Ok(true);
        }
    }
    Ok(false)
}
