//! Lokale Bestaetigungs-IPC ohne Adapterzugriff.
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::zustand::InterfaceState;

#[derive(Debug, Clone, Deserialize, Serialize, Type)]
pub enum UtteranceInput {
    Started,
    // Tauri deserialisiert den gesamten String vor dem Aufruf; dies ist kein IPC-Speicherlimit.
    // utterance prueft zuerst die Laenge und verwirft grosse Texte vor jeder Sperre.
    Final { text: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
pub enum Decision {
    Wartet,
    Rueckfrage,
    Freigegeben,
    Abgelehnt,
    Abgelaufen,
    Ungueltig,
}

impl From<super::aeusserung::Entscheidung> for Decision {
    fn from(entscheidung: super::aeusserung::Entscheidung) -> Self {
        use super::aeusserung::Entscheidung as E;
        match entscheidung {
            E::Wartet => Self::Wartet,
            E::Rueckfrage => Self::Rueckfrage,
            E::Freigegeben => Self::Freigegeben,
            E::Abgelehnt => Self::Abgelehnt,
            E::Abgelaufen => Self::Abgelaufen,
            E::Ungueltig => Self::Ungueltig,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub enum InterfaceError {
    KennungUngueltig,
    Widerspruch,
    ZuViele,
    Beendet,
    Verweigert,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub enum CallResult {
    Completed {
        outcome: String,
        /// JSON-Text des gerahmten Ergebnisses (specta kennt `serde_json::Value` nicht).
        data: Option<String>,
        message: Option<String>,
    },
    AwaitingConfirmation {
        question_id: String,
    },
}

pub(super) fn als_json(wert: super::kodierung::Wert) -> serde_json::Value {
    use super::kodierung::Wert;
    use serde_json::Value;
    match wert {
        Wert::Null => Value::Null,
        Wert::Bool(wert) => Value::Bool(wert),
        Wert::Zahl(wert) => wert
            .parse::<serde_json::Number>()
            .map_or_else(|_| Value::String(wert), Value::Number),
        Wert::Text(wert) => Value::String(wert),
        Wert::Liste(werte) => Value::Array(werte.into_iter().map(als_json).collect()),
        Wert::Objekt(werte) => {
            Value::Object(werte.into_iter().map(|(k, v)| (k, als_json(v))).collect())
        }
    }
}

impl From<super::aufrufkern::Aufrufergebnis> for CallResult {
    fn from(ergebnis: super::aufrufkern::Aufrufergebnis) -> Self {
        let outcome = match ergebnis.ergebnis {
            wort @ ("ok"
            | "nicht_vorhanden"
            | "abgelehnt"
            | "abgebrochen"
            | "zeitlimit"
            | "netzfehler"
            | "http_fehler"
            | "umleitung"
            | "unklar"
            | "zu_gross"
            | "antwort_ungueltig"
            | "adresse_abgewiesen"
            | "bestaetigung_abgelehnt"
            | "bestaetigung_abgelaufen") => wort,
            _ => "netzfehler",
        };
        Self::Completed {
            outcome: outcome.into(),
            data: ergebnis.daten.map(|wert| als_json(wert).to_string()),
            message: ergebnis.meldung,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub(crate) fn voice_call_interface_begin(
    state: tauri::State<'_, Arc<InterfaceState>>,
    conversation_id: String,
    answer_id: String,
    model_request_id: String,
) -> Result<(), InterfaceError> {
    state.begin(&conversation_id, &answer_id, &model_request_id)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn voice_call_interface_call(
    state: tauri::State<'_, Arc<InterfaceState>>,
    conversation_id: String,
    answer_id: String,
    model_request_id: String,
    call_id: String,
    tool_name: String,
    arguments_json: String,
) -> Result<CallResult, InterfaceError> {
    let state = Arc::clone(state.inner());
    tokio::task::spawn_blocking(move || {
        state.call(
            &conversation_id,
            &answer_id,
            &model_request_id,
            &call_id,
            &tool_name,
            &arguments_json,
        )
    })
    .await
    .map_err(|_| InterfaceError::Verweigert)?
}

#[tauri::command]
#[specta::specta]
pub(crate) fn voice_call_interface_end(
    state: tauri::State<'_, Arc<InterfaceState>>,
    conversation_id: String,
) -> Result<(), InterfaceError> {
    state.end(&conversation_id)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn voice_call_interface_utterance(
    state: tauri::State<'_, Arc<InterfaceState>>,
    conversation_id: String,
    answer_id: String,
    model_request_id: String,
    question_id: String,
    utterance_id: String,
    input: UtteranceInput,
) -> Result<Decision, InterfaceError> {
    state.utterance(
        &conversation_id,
        &answer_id,
        &model_request_id,
        &question_id,
        &utterance_id,
        input,
    )
}
