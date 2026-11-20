//! KI-Stufe „Rechtschreibung und Grammatik“ der Textaufbereitung (lva#712, A3).
//!
//! Das Modell liefert unzuverlässige Vorschläge, nie Änderungen: jeder Vorschlag wird als
//! `Aenderung` mit `art = Sprache` gegen den eingefrorenen Textstand gebildet und bleibt
//! einzeln annehmbar. Dieser Teil (PR 1) hält die Verträge: Textstand, Positionen, Gruppen,
//! Konflikte, Cache, Versandpolitik und den Modellzugang samt Mock. Parser, Maskierung,
//! Validierung, Abschnittsjob und Testkorpus folgen in PR 2.
//!
//! Datenschutz: kein Text, Prompt oder Modellantwort in Logs, Fehlern oder Berichten; nur
//! feste Codes und Zähler ([`fehler`]).
#![allow(dead_code)]

pub mod cache;
pub mod fehler;
pub mod konflikt;
pub mod modell;
pub mod politik;
pub mod vertrag;
