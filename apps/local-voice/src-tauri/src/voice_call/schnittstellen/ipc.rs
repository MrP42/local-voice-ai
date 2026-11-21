//! Lokale Bestaetigungs-IPC ohne Adapterzugriff.
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::zustand::InterfaceState;

#[derive(Debug, Clone, Deserialize, Serialize, Type)]
pub enum UtteranceInput {
    Started,
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
    Completed { outcome: String },
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
pub(crate) fn voice_call_interface_call(
    state: tauri::State<'_, Arc<InterfaceState>>,
    conversation_id: String,
    answer_id: String,
    model_request_id: String,
    call_id: String,
    tool_name: String,
    arguments_json: String,
) -> Result<CallResult, InterfaceError> {
    // Inhalte werden weder ausgewertet noch protokolliert.
    let _ = (tool_name, arguments_json);
    state.call(&conversation_id, &answer_id, &model_request_id, &call_id)
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
