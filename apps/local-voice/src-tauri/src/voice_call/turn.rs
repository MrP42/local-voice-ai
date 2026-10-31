//! Ein Durchgang eines Modells für den Orchestrator des Sprachgesprächs (Stufe 2).
//!
//! `voice_call_chat` zerlegt die Antwort sofort in sprechbare Häppchen und schickt sie an die
//! Stimme. Der Orchestrator (`src/components/voice-call/orchestration.ts`) braucht stattdessen den
//! rohen Textstrom: er erkennt Werkzeugaufrufe, spricht Überbrückungen und entscheidet selbst,
//! was gesprochen wird. `voice_call_turn` liefert deshalb Textstücke (`kind: "delta"`) und am Ende
//! den Verbrauch (`kind: "usage"`), nichts davon geht ungefragt an die Stimme.
//!
//! Das Modell wählt `model_id` (Kennung aus den Einstellungen, `llm_models`); der Aufrufer nennt es
//! immer ausdrücklich, auch für den Sprecher. Der Denker ist so ein zweites Modell, ohne die
//! globale Auswahl zu ändern.
//!
//! # Souveränität (hier durchgesetzt, nicht nur in der Oberfläche)
//! `stufe` ist `nur_lokal`, `eu` oder `frei`. Bei `nur_lokal` lehnt der Befehl jedes Cloud-Modell ab,
//! bei `eu` jedes Cloud-Modell, dessen Verarbeitungsländer nicht alle in der EU liegen (oder unbekannt
//! sind). Unbekannte Stufen werden abgelehnt. Es wird dann nichts gesendet.
//!
//! # Fehlerfälle und ihre Absicherung
//!
//! | # | Fehlerfall | Verhalten | Absicherung |
//! |---|---|---|---|
//! | 1 | Kennung unbekannt, Modell oder Verbindung abgeschaltet | Fehlertext, kein Aufruf | `unknown_model_is_refused` |
//! | 5 | Cloud-Modell bei „nur lokal“/„eu“, unbekannte Stufe | abgelehnt, nichts gesendet | `sovereignty_is_enforced` |
//! | 6 | Abbruch oder Fehler nach schon verbrauchten Token | Ereignis `usage` kommt trotzdem | wie `voice_call_chat` |
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

/// Ländercodes der EU (ISO 3166-1 alpha-2; „EU“ für EU-weit).
const EU_COUNTRIES: [&str; 28] = [
    "AT", "BE", "BG", "HR", "CY", "CZ", "DK", "EE", "FI", "FR", "DE", "GR", "HU", "IE", "IT", "LV",
    "LT", "LU", "MT", "NL", "PL", "PT", "RO", "SK", "SI", "ES", "SE", "EU",
];

/// Ob die Souveränitätsstufe ein Modell zulässt (`cloud`, Verarbeitungsländer).
pub fn sovereignty_allows(stufe: &str, cloud: bool, countries: &[String]) -> Result<(), String> {
    let refused = || {
        Err(format!(
            "Das Modell ist bei Souveränität „{stufe}“ nicht zulässig. Es wurde nichts gesendet."
        ))
    };
    match stufe {
        "frei" => Ok(()),
        "nur_lokal" if !cloud => Ok(()),
        "eu" if !cloud => Ok(()),
        "eu" if !countries.is_empty()
            && countries
                .iter()
                .all(|c| EU_COUNTRIES.contains(&c.to_ascii_uppercase().as_str())) =>
        {
            Ok(())
        }
        "nur_lokal" | "eu" => refused(),
        _ => Err("Unbekannte Souveränitätsstufe. Es wurde nichts gesendet.".to_string()),
    }
}

/// Anbieter, Modell und Schlüssel zu einer Modellkennung unter einer Souveränitätsstufe.
fn resolve(
    settings: &AppSettings,
    model_id: &str,
    stufe: &str,
) -> Result<ResolvedProvider, String> {
    let mut chosen = settings.clone();
    chosen.llm_active_model_id = Some(model_id.to_string());
    let assessment = {
        let (connection, _) = chosen
            .active_llm_model()
            .ok_or_else(|| "Das gewählte Modell ist nicht verfügbar.".to_string())?;
        crate::commands::compliance::assess_connection(&chosen, connection)
    };
    sovereignty_allows(stufe, assessment.cloud, &assessment.countries)?;
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
    model_id: String,
    stufe: String,
) -> Result<String, String> {
    // Ein vorgezogener Abbruch wird zuerst eingelöst, auch wenn unten etwas scheitert.
    if EARLY_CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&request_id)
    {
        return Ok(String::new());
    }
    let settings = crate::settings::get_settings(&app);
    let (provider, model, api_key) = resolve(&settings, &model_id, &stufe)?;
    let messages: Vec<StreamMessage> = messages
        .into_iter()
        .map(|m| StreamMessage::new(&m.role, m.content))
        .collect();
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

    // Der Verbrauch wird in jedem Ausgang gemeldet (auch bei Abbruch und Fehler), sofern etwas
    // erfasst wurde.
    if !captured.is_empty() {
        let _ = app.emit(
            EVENT,
            serde_json::json!({
                "kind": "usage",
                "request_id": request_id,
                "usage": usage_payload(&captured),
            }),
        );
    }
    match result {
        None => Ok(String::new()),
        Some(Ok(full)) => Ok(full),
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

    fn c(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unknown_model_is_refused() {
        let settings = crate::settings::get_default_settings();
        let err = resolve(&settings, "gibt-es-nicht:modell", "frei").unwrap_err();
        assert!(err.contains("nicht verfügbar"));
    }

    #[test]
    fn sovereignty_is_enforced() {
        // nur lokal: nur Modelle auf diesem Rechner.
        assert!(sovereignty_allows("nur_lokal", false, &[]).is_ok());
        assert!(sovereignty_allows("nur_lokal", true, &c(&["DE"])).is_err());
        assert!(sovereignty_allows("nur_lokal", true, &[]).is_err());
        // eu: lokal, oder Cloud mit ausschließlich EU-Ländern; unbekannte Region wird abgelehnt.
        assert!(sovereignty_allows("eu", false, &[]).is_ok());
        assert!(sovereignty_allows("eu", true, &c(&["DE", "fr"])).is_ok());
        assert!(sovereignty_allows("eu", true, &c(&["EU"])).is_ok());
        assert!(sovereignty_allows("eu", true, &c(&["DE", "US"])).is_err());
        assert!(sovereignty_allows("eu", true, &[]).is_err());
        // frei: alles.
        assert!(sovereignty_allows("frei", true, &c(&["US"])).is_ok());
        // Unbekannte oder leere Stufe: abgelehnt.
        for stufe in ["", "FREI", "egal", "nur lokal"] {
            assert!(sovereignty_allows(stufe, false, &[]).is_err(), "{stufe:?}");
        }
        let msg = sovereignty_allows("nur_lokal", true, &c(&["US"])).unwrap_err();
        assert!(msg.contains("nichts gesendet"));
    }
}
