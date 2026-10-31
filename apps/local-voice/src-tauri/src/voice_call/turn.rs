//! Ein Durchgang eines Modells für den Orchestrator des Sprachgesprächs (Stufe 2).
//!
//! `voice_call_chat` zerlegt die Antwort sofort in sprechbare Häppchen und schickt sie an die
//! Stimme. Der Orchestrator (`src/components/voice-call/orchestration.ts`) braucht stattdessen den
//! rohen Textstrom: er erkennt Werkzeugaufrufe, spricht Überbrückungen und entscheidet selbst,
//! was gesprochen wird. `voice_call_turn` liefert deshalb Textstücke (`kind: "delta"`) und am Ende
//! den Verbrauch (`kind: "usage"`), nichts davon geht ungefragt an die Stimme.
//!
//! Das Modell wählt `model_id` (Kennung aus den Einstellungen, `llm_models`); ohne Angabe gilt das
//! aktive Sprachmodell. Der Denker ist so ein zweites Modell, ohne die globale Auswahl zu ändern.
//!
//! # Fehlerfälle und ihre Absicherung
//!
//! | # | Fehlerfall | Verhalten | Absicherung |
//! |---|---|---|---|
//! | 1 | Kennung unbekannt, Modell oder Verbindung abgeschaltet | Fehlertext, kein Aufruf | `unknown_model_is_refused` |
//! | 2 | Abbruch (`voice_call_cancel` mit der Kennung des Durchgangs) | endet mit `Ok("")`, nichts mehr gesendet | wie `voice_call_chat` |
//! | 3 | Abbruch vor dem Start (IPC-Reihenfolge) | `EARLY_CANCELS` fängt ihn | wie `voice_call_chat` |
//! | 4 | Modellfehler | Fehlertext zurück (ohne Inhalte), Ereignis `failed` | wie `voice_call_chat` |
//!
//! Texte stehen nie im Log.

use std::sync::Arc;

use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

use super::{usage_payload, VoiceCallMessage, CANCELS, EARLY_CANCELS, EVENT};
use crate::llm_client::{self, StreamMessage};
use crate::managers::meetings::llm_call::ResolvedProvider;
use crate::managers::usage::{self, Purpose};
use crate::settings::AppSettings;

/// Anbieter, Modell und Schlüssel zu einer Modellkennung (leer: aktives Sprachmodell).
fn resolve(settings: &AppSettings, model_id: Option<&str>) -> Result<ResolvedProvider, String> {
    let Some(id) = model_id.filter(|id| !id.is_empty()) else {
        return crate::managers::meetings::llm_call::resolve_provider(settings);
    };
    let mut chosen = settings.clone();
    chosen.llm_active_model_id = Some(id.to_string());
    if chosen.active_llm_model().is_none() {
        return Err("Das gewählte Modell ist nicht verfügbar.".to_string());
    }
    chosen.sync_legacy_from_llm();
    crate::managers::meetings::llm_call::resolve_provider(&chosen)
}

/// Ein Durchgang eines Modells als roher Textstrom (Ereignisse `delta`, am Ende `usage`).
#[tauri::command]
#[specta::specta]
pub async fn voice_call_turn(
    app: AppHandle,
    request_id: String,
    messages: Vec<VoiceCallMessage>,
    model_id: Option<String>,
) -> Result<String, String> {
    let settings = crate::settings::get_settings(&app);
    let (provider, model, api_key) = resolve(&settings, model_id.as_deref())?;
    let messages: Vec<StreamMessage> = messages
        .into_iter()
        .map(|m| StreamMessage::new(&m.role, m.content))
        .collect();

    if EARLY_CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&request_id)
    {
        return Ok(String::new());
    }
    let cancel = Arc::new(Notify::new());
    CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(request_id.clone(), cancel.clone());

    let on_delta = {
        let app = app.clone();
        let request_id = request_id.clone();
        move |delta: &str| {
            let _ = app.emit(
                EVENT,
                serde_json::json!({"kind": "delta", "request_id": request_id, "text": delta}),
            );
        }
    };
    let (result, captured) = usage::with_capture(async {
        let call = llm_client::send_chat_completion_stream(
            Purpose::Chat,
            &provider,
            api_key,
            &model,
            messages,
            &on_delta,
        );
        let result = tokio::select! {
            r = call => Some(r),
            _ = cancel.notified() => None,
        };
        (result, usage::captured_calls().0)
    })
    .await;
    CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&request_id);

    match result {
        None => Ok(String::new()),
        Some(Ok(full)) => {
            let _ = app.emit(
                EVENT,
                serde_json::json!({
                    "kind": "usage",
                    "request_id": request_id,
                    "usage": usage_payload(&captured),
                }),
            );
            Ok(full)
        }
        Some(Err(e)) => {
            let _ = app.emit(
                EVENT,
                serde_json::json!({"kind": "failed", "request_id": request_id}),
            );
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_model_is_refused() {
        let settings = crate::settings::get_default_settings();
        let err = resolve(&settings, Some("gibt-es-nicht:modell")).unwrap_err();
        assert!(err.contains("nicht verfügbar"));
    }
}
