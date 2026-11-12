//! Globaler Abrufindex des Sprachassistenten (Plan
//! `docs/plans/2026-11-05_globaler-sprachassistent.md`, Paket P1).
//!
//! „Voller Kontext“ heißt hier: ein Suchindex über alle freigegebenen Inhalte
//! der App (Bücher bis zur Lesegrenze, Buchnotizen und KI-Dokumente,
//! Besprechungen, Diktate, frühere Gespräche, Vorlesen-Seiten). Pro Frage
//! liefert [`retrieve::retrieve`] die besten Auszüge mit Belegmarken `[Q<k>]`;
//! nur diese gehen in den Prompt, nie alles.
//!
//! Das Modul ist rein (keine App, keine Datenbank, keine Uhr): die App-Schicht
//! liefert Dokumente ([`model::ContextDoc`]), Freigaben und Lesegrenzen. Die
//! Tokenisierung und die BM25-Formel sind dieselben wie im Buch-Gespräch
//! (`book_ai::talk::{lex, retrieve}`), damit beide Wege gleich suchen. Bücher
//! werden mit `book_ai::talk::index::build` zerlegt (Seiten, Kapitel,
//! UTF-16-Offsets für den Sprung im Reader).
//!
//! # Grenzen
//! - Buchtext nur bis zur Lesegrenze `frontier` (UTF-16, Original); ein Stück,
//!   das die Grenze überspannt, wird an der Satzgrenze davor gekürzt und NUR
//!   mit dem gekürzten Text bewertet (kein Durchsickern späterer Wörter über
//!   die Rangfolge). „Ganzes Buch“ nur, wenn die Anfrage es ausdrücklich
//!   erlaubt (`whole_books`).
//! - Inhalte sind Daten, keine Anweisungen; sie stehen nie im Log.
//! - Der Index ist inkrementell: unveränderte Dokumente (gleicher Hash) werden
//!   nicht neu zerlegt.

pub mod berechtigung;
pub mod chunk;
pub mod index;
pub mod model;
pub mod progress;
pub mod prompt;
pub mod retrieve;
pub mod service;

#[cfg(test)]
mod eval_tests;
#[cfg(test)]
mod tests;
