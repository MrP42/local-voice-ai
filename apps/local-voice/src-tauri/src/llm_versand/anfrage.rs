//! Anfrage und Antwort von [`versenden`](super::versenden) (lva#604, Teilpaket 3).
//!
//! [`Anfrage`] trägt alles, was ein Versandversuch außer dem Ziel, den Quellen und der
//! Freigabequelle braucht: den Zweck (für das Verbrauchs-Ledger), den Anbieter (für Compliance,
//! Budget und Kopfzeilen) und den Inhalt. Es gibt genau zwei Arten Inhalt: eine Antwort am
//! Stück (blockierend, optional mit JSON-Schema) und eine Antwort als Strom mit
//! Delta-Rückruf.

use serde_json::Value;

use crate::llm_client::{ReasoningConfig, StreamLimits, StreamMessage};
use crate::managers::usage::Purpose;
use crate::settings::PostProcessProvider;

/// Inhalt einer Antwort am Stück (blockierend).
#[derive(Debug, Clone)]
pub struct ChatInhalt {
    /// Die Nutzernachricht.
    pub nutzer: String,
    /// Systemnachricht, wenn vorhanden.
    pub system: Option<String>,
    /// JSON-Schema für strukturierte Ausgabe (HTTP: `response_format`, CLI: im Systemtext).
    pub schema: Option<Value>,
    /// OpenAI-Stil: oberstes Feld `reasoning_effort`.
    pub reasoning_effort: Option<String>,
    /// OpenRouter-Stil: verschachteltes Objekt `reasoning`.
    pub reasoning: Option<ReasoningConfig>,
}

/// Was gesendet wird.
pub enum Inhalt<'a> {
    /// Eine Antwort am Stück, mit oder ohne JSON-Schema.
    Chat(ChatInhalt),
    /// Eine Antwort als Strom: `on_delta` bekommt jedes Textstück in Reihenfolge.
    ///
    /// Bei einem Cloud-Ziel mit `Quellen` ungleich `keine()` gehen **alle** Inhaltsausgaben
    /// (Deltas, Denktexte, CLI-Denktexte) erst nach bestandenem Abschlussvergleich an den
    /// Rückruf; bei `Gesperrt` kommt nichts an.
    Strom {
        /// Gesprächsverlauf (`system`, `user`, `assistant`).
        nachrichten: Vec<StreamMessage>,
        /// Ausgabegrenze und Strenge.
        grenzen: StreamLimits,
        /// Empfänger der Textstücke.
        on_delta: &'a (dyn Fn(&str) + Send + Sync),
    },
}

/// Ein Modellaufruf: Zweck, Anbieter, Zugangsschlüssel und Inhalt.
pub struct Anfrage<'a> {
    /// Wofür der Aufruf war (Verbrauchs-Ledger).
    pub zweck: Purpose,
    /// Der Anbieter, dessen Regeln (Compliance, Budget) und Kopfzeilen gelten. Die **Adresse**
    /// kommt aus dem [`Ziel`](super::Ziel), nie von hier.
    pub anbieter: &'a PostProcessProvider,
    /// Zugangsschlüssel des Anbieters (leer bei lokalen und CLI-Zielen).
    pub api_key: String,
    /// Der Inhalt.
    pub inhalt: Inhalt<'a>,
}

/// Die Antwort eines erfolgreichen Versands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Antwort {
    /// Der Antworttext. Bei einem Strom immer `Some` (der ganze Text); bei einer Antwort am
    /// Stück `None`, wenn der Anbieter keinen Inhalt lieferte.
    pub text: Option<String>,
    /// Der Server hat die Antwort abgeschnitten (`finish_reason == "length"`). Ein
    /// abgeschnittenes JSON ist nie gültig; wer das weiß, verkleinert den Prompt.
    pub abgeschnitten: bool,
}

/// Die Antwort eines rohen JSON-POST ([`uebergang::json_post`](super::uebergang)): für Aufrufer mit
/// eigener Auswertung (Agent, Einbettungen, Tokenzählung). Der Transport klassifiziert nur; der
/// Text einer Fehlerantwort wird gelesen und verworfen.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonAntwort {
    /// HTTP-Status.
    pub status: u16,
    /// Die Fehlerantwort meldete einen Kontextüberlauf (nur bei nicht erfolgreichem Status).
    pub ueberlauf: bool,
    /// Der JSON-Körper einer erfolgreichen Antwort; `None` bei Fehlerstatus und bei Körpern,
    /// die kein JSON sind oder abbrachen.
    pub json: Option<Value>,
}

impl JsonAntwort {
    /// Status 2xx?
    pub fn erfolgreich(&self) -> bool {
        (200..300).contains(&self.status)
    }
}
