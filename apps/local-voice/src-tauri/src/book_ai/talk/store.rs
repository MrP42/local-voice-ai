//! Verlauf des Buch-Gesprächs: `ki/gespraech.json` beim Buch, höchstens
//! `MAX_TURNS` Wechsel (Form `StoredTurn` des Vertrags). Jeder Wechsel trägt
//! Bereich und Spoilergrenze, mit der er entstand; in den Prompt gehen nur
//! Wechsel, die nicht über die aktuelle Grenze hinaus wissen. Abgebrochene
//! Antworten werden nie gespeichert.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

use super::prompt::{eligible_history, Turn};
use super::{BookTalkAnswer, BookTalkInput, BookTalkScope};
use crate::book_ai::store::ki_dir;
use crate::commands::pages::write_atomic;

pub const MAX_TURNS: usize = 200;

/// Verlaufsänderungen laufen nacheinander (Lesen, Ändern, Schreiben).
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Ein gespeicherter Wechsel (Form wie `StoredTurn` im Vertrag).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct StoredTurn {
    pub question: String,
    pub answer: BookTalkAnswer,
    pub scope: BookTalkScope,
    /// Spoilergrenze (UTF-16 im Originaltext) zum Zeitpunkt der Frage.
    pub frontier: u32,
    pub input: BookTalkInput,
    /// Zeitpunkt (RFC 3339, UTC).
    pub at: String,
}

fn path(page_dir: &Path) -> PathBuf {
    ki_dir(page_dir).join("gespraech.json")
}

pub fn load(page_dir: &Path) -> Result<Vec<StoredTurn>, String> {
    match std::fs::read_to_string(path(page_dir)) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|_| "book_talk:history_broken".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err("book_talk:history_unreadable".to_string()),
    }
}

fn save(page_dir: &Path, turns: &[StoredTurn]) -> Result<(), String> {
    let raw =
        serde_json::to_string(turns).map_err(|_| "book_talk:history_unwritable".to_string())?;
    crate::book_ai::store::im_seitenordner(page_dir, || {
        let dir = ki_dir(page_dir);
        crate::book_ai::store::unterordner_anlegen(page_dir, &dir)?;
        write_atomic(&path(page_dir), raw.as_bytes())
    })
    .map_err(|_| "book_talk:history_unwritable".to_string())
}

/// Wechsel anhängen; die ältesten über `MAX_TURNS` fallen weg.
pub fn append(page_dir: &Path, turn: StoredTurn) -> Result<(), String> {
    let _guard = lock();
    let mut turns = load(page_dir).unwrap_or_default();
    turns.push(turn);
    if turns.len() > MAX_TURNS {
        let drop = turns.len() - MAX_TURNS;
        turns.drain(..drop);
    }
    save(page_dir, &turns)
}

pub fn clear(page_dir: &Path) -> Result<(), String> {
    let _guard = lock();
    match std::fs::remove_file(path(page_dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("book_talk:history_unwritable".to_string()),
    }
}

impl StoredTurn {
    /// Der Wechsel in der Form für den Prompt (Antworttext, Bereich, Grenze).
    pub fn prompt_turn(&self) -> Turn {
        Turn {
            question: self.question.clone(),
            answer: self.answer.text.clone(),
            scope: self.scope,
            frontier: self.frontier,
        }
    }
}

/// Frühere Wechsel für den Prompt: im Bereich „bis hier“ nur Wechsel, die
/// selbst „bis hier“ galten und deren Grenze nicht über der aktuellen liegt
/// (sonst trüge der Verlauf Wissen von hinter der Grenze ein); im Bereich
/// „ganzes Buch“ alle. Die letzten `MAX_HISTORY_TURNS`, ältester zuerst.
pub fn history_for_prompt(turns: &[StoredTurn], scope: BookTalkScope, frontier: u32) -> Vec<Turn> {
    let all: Vec<Turn> = turns.iter().map(StoredTurn::prompt_turn).collect();
    eligible_history(&all, scope, frontier)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::book_ai::talk::intent::BookTalkIntent;

    pub(crate) fn turn(q: &str, a: &str, scope: BookTalkScope, frontier: u32) -> StoredTurn {
        StoredTurn {
            question: q.into(),
            answer: BookTalkAnswer {
                intent: BookTalkIntent::Question,
                sentences: Vec::new(),
                text: a.into(),
                speech: a.into(),
                citations: Vec::new(),
                found_only: false,
                reason: None,
                dropped: 0,
                scope,
                frontier,
                preview: false,
            },
            scope,
            frontier,
            input: BookTalkInput::Text,
            at: "2026-11-05T18:00:00Z".into(),
        }
    }

    #[test]
    fn history_follows_scope_and_frontier() {
        use BookTalkScope::{Book, UntilPosition as Until};
        let turns = [
            turn("alt-bis-100", "A1", Until, 100),
            turn("buch", "A2", Book, 900),
            turn("spaeter-bis-900", "A3", Until, 900),
        ];
        let q = |h: Vec<Turn>| h.into_iter().map(|t| t.question).collect::<Vec<_>>();
        // „bis hier“ bei 500: nur der Wechsel bis 100.
        assert_eq!(q(history_for_prompt(&turns, Until, 500)), ["alt-bis-100"]);
        // Zurückgescrollt oder unbekannt (0): keiner kennt weniger.
        assert!(history_for_prompt(&turns, Until, 50).is_empty());
        assert!(history_for_prompt(&turns, Until, 0).is_empty());
        // Nach dem Wechsel „Ganzes Buch“ → „Bis hier“ fällt der Ganz-Buch-Wechsel aus.
        assert_eq!(
            q(history_for_prompt(&turns, Until, 900)),
            ["alt-bis-100", "spaeter-bis-900"]
        );
        // Ganzes Buch: die letzten zwei.
        assert_eq!(
            q(history_for_prompt(&turns, Book, 0)),
            ["buch", "spaeter-bis-900"]
        );
    }

    #[test]
    fn the_history_is_kept_to_200_turns_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..205 {
            append(
                dir.path(),
                turn(&format!("f{i}"), "a", BookTalkScope::Book, 0),
            )
            .unwrap();
        }
        let turns = load(dir.path()).unwrap();
        assert_eq!(turns.len(), MAX_TURNS);
        assert_eq!(turns[0].question, "f5");
        clear(dir.path()).unwrap();
        assert!(load(dir.path()).unwrap().is_empty());
        clear(dir.path()).unwrap();
    }
}
