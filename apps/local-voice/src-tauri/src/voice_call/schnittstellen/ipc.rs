//! Verweigernde IPC-Grundlage ohne Freigaben oder Adapterzugriff.
use std::sync::Arc;

use serde::Serialize;
use specta::Type;

use super::zustand::InterfaceState;

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
