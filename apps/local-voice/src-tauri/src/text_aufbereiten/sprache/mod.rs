//! KI-Stufe „Rechtschreibung und Grammatik“ der Textaufbereitung (lva#712, A3).
//!
//! Das Modell liefert unzuverlässige Vorschläge, nie Änderungen: jeder Vorschlag wird als
//! `Aenderung` mit `art = Sprache` gegen den eingefrorenen Textstand gebildet und bleibt
//! einzeln annehmbar. Dieser Teil (PR 1) hält die Verträge: Textstand, Positionen, Gruppen,
//! Konflikte, Cache, Versandpolitik und den Modellzugang samt Mock; PR 2 ergaenzt Parser,
//! Maskierung, Validierung, Wort-Diff, Abschnittsjob und den Testkorpus.
//!
//! Datenschutz: kein Text, Prompt oder Modellantwort in Logs, Fehlern oder Berichten; nur
//! feste Codes und Zähler ([`fehler`]).
#![allow(dead_code)]

pub mod cache;
pub mod diff;
pub mod fehler;
pub mod job;
pub mod konflikt;
#[cfg(test)]
mod korpus;
pub mod maske;
pub mod modell;
pub mod parser;
pub mod politik;
pub mod prompt;
pub mod pruefung;
pub mod vertrag;
