//! Sprachgespräch mit der KI (Echtzeit, wie ein Telefonat).
//!
//! Die Gesprächsführung (Zuhören, Satzende erkennen, Unterbrechen, Wiedergabe)
//! liegt im Frontend (`src/components/voice-call/`): nur dort läuft die
//! Echo-Unterdrückung des Browsers, und sie braucht die Wiedergabe im selben
//! Audiokontext wie das Mikrofon. Dieses Modul stellt die vier Bausteine, die
//! nur der Kern kann:
//!
//! * `voice_call_start`: Sprach-, Stimm- und Sprachmodell vorwärmen, damit die
//!   erste Antwort nicht auf einen Modellstart wartet.
//! * `voice_call_transcribe`: ein Stück Sprache (16 kHz, mono, PCM16) erkennen.
//! * `voice_call_chat`: Antwort des aktiven Sprachmodells als Strom, schon
//!   beim Eintreffen in sprechbare Häppchen zerlegt (`sentences`).
//! * `turn::voice_call_turn`: ein Durchgang eines Modells als roher Textstrom für den
//!   Orchestrator (Stufe 2), wahlweise mit einem zweiten Modell (Denker).
//! * `voice_call_synthesize`: ein Häppchen mit der eingestellten Stimme
//!   (Standard: Anton) zu WAV.
//!
//! Audio geht als Base64 über die Schnittstelle: das passt zur gewöhnlichen
//! Command-Schicht (specta) und ist für Gesprächslängen von Sekunden klein genug.
//! Texte und Audio stehen nie im Log.

pub mod sentences;
pub mod turn;

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::llm_client::{self, StreamMessage};
use crate::managers::transcription::TranscriptionManager;
use crate::managers::tts::TtsManager;
use crate::managers::usage::{self, CapturedCall, Purpose};
use sentences::SentenceSplitter;

/// Name des Ereignisses, über das Antwortstücke zum Frontend laufen.
pub const EVENT: &str = "voice-call";

static CANCELS: Lazy<Mutex<HashMap<String, Arc<Notify>>>> = Lazy::new(Default::default);
/// Abbrüche, die vor dem Start ihrer Anfrage eintrafen (IPC-Reihenfolge ist nicht garantiert).
static EARLY_CANCELS: Lazy<Mutex<std::collections::HashSet<String>>> = Lazy::new(Default::default);

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct VoiceCallReady {
    /// Anzeigename des Sprachmodells (`Anbieter · Modell`), leer wenn keins eingerichtet ist.
    pub llm: String,
    /// Lokales Sprachmodell (kostenlos, aber CPU/GPU-lastig).
    pub llm_local: bool,
    /// Hinweise, die das Gespräch verlangsamen würden (nichts Hartes).
    pub warnings: Vec<String>,
    /// Dauer des Vorwärmens in Millisekunden.
    pub warmup_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct VoiceCallMessage {
    /// `system` | `user` | `assistant`
    pub role: String,
    pub content: String,
}

fn pcm16_b64_to_f32(b64: &str) -> Result<Vec<f32>, String> {
    let bytes = B64
        .decode(b64)
        .map_err(|e| format!("Audio ungültig: {e}"))?;
    if bytes.len() % 2 != 0 {
        return Err("Audio ungültig: ungerade Bytezahl".into());
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect())
}

/// Verbrauch der erfassten Modellaufrufe für das `done`-Ereignis: Summe der
/// Token, die Ereignisnummern des Ledgers als Liste (`usage_event_ids`; die
/// erste zusätzlich als `usage_event_id`), Anbieter und ob er lokal läuft.
fn usage_payload(calls: &[CapturedCall]) -> serde_json::Value {
    let ids: Vec<i64> = calls.iter().filter_map(|c| c.usage_event_id).collect();
    serde_json::json!({
        "prompt_tokens": calls.iter().map(|c| c.prompt_tokens).sum::<u64>(),
        "completion_tokens": calls.iter().map(|c| c.completion_tokens).sum::<u64>(),
        "usage_event_id": ids.first(),
        "usage_event_ids": ids,
        "provider_id": calls.last().map(|c| c.provider_id.clone()),
        "local": calls.last().map(|c| c.local).unwrap_or(false),
    })
}

/// Wärmt STT, TTS und LLM vor und meldet, was das Gespräch verlangsamen würde.
#[tauri::command]
#[specta::specta]
pub async fn voice_call_start(app: AppHandle) -> Result<VoiceCallReady, String> {
    let started = Instant::now();
    let settings = crate::settings::get_settings(&app);
    let mut warnings = Vec::new();

    let (llm, llm_local) = match crate::managers::meetings::llm_call::resolve_provider(&settings) {
        Ok((provider, model, _)) => (
            format!("{} · {}", provider.label, model),
            crate::managers::llm::is_local(&provider),
        ),
        Err(e) => {
            warnings.push(e);
            (String::new(), false)
        }
    };

    if settings.model_unload_timeout == crate::settings::ModelUnloadTimeout::Immediately {
        warnings.push(
            "Das Spracherkennungsmodell wird nach jeder Eingabe entladen (Einstellungen → Eingabe). Das kostet im Gespräch jedes Mal Sekunden; „Nie“ oder „2 Minuten“ wählen."
                .into(),
        );
    }

    let stt = app.state::<Arc<TranscriptionManager>>().inner().clone();
    let model = settings.selected_model.clone();
    let stt_ready = tokio::task::spawn_blocking(move || {
        stt.initiate_model_load();
        stt.wait_until_loaded(&model, Duration::from_secs(90))
    })
    .await
    .unwrap_or(false);
    if !stt_ready {
        return Err("Das Spracherkennungsmodell konnte nicht geladen werden (Einstellungen → Eingabe → Modell).".into());
    }

    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.refresh_from_settings();
    tts.ensure_engine_ready()
        .await
        .map_err(|e| format!("Die Stimme ist nicht bereit: {e}"))?;

    Ok(VoiceCallReady {
        llm,
        llm_local,
        warnings,
        warmup_ms: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
    })
}

/// Erkennt ein Stück Sprache. `pcm16_b64` = 16 kHz, mono, 16 Bit little endian.
#[tauri::command]
#[specta::specta]
pub async fn voice_call_transcribe(app: AppHandle, pcm16_b64: String) -> Result<String, String> {
    let samples = pcm16_b64_to_f32(&pcm16_b64)?;
    let stt = app.state::<Arc<TranscriptionManager>>().inner().clone();
    tokio::task::spawn_blocking(move || stt.transcribe(samples).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
        .map(|t| t.trim().to_string())
}

/// Antwort des aktiven Sprachmodells als Strom. Jedes Häppchen kommt sofort als
/// Ereignis `voice-call` (`{kind: "sentence", request_id, index, text}`), die
/// fertige Antwort ist der Rückgabewert. Bricht `voice_call_cancel` ab, endet
/// der Aufruf mit `Ok` und leerem Text.
#[tauri::command]
#[specta::specta]
pub async fn voice_call_chat(
    app: AppHandle,
    request_id: String,
    messages: Vec<VoiceCallMessage>,
    model_id: Option<String>,
    effort: Option<String>,
) -> Result<String, String> {
    // Ein vorgezogener Abbruch wird zuerst eingelöst, damit er auch bei einem Fehler unten
    // nicht ewig liegen bleibt.
    if EARLY_CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&request_id)
    {
        return Ok(String::new());
    }
    // Sprecher aus dem Chatfeld (leer = globales Modell, wie bisher).
    let settings = turn::with_speaker(
        &crate::settings::get_settings(&app),
        model_id.as_deref(),
        effort.as_deref(),
    );
    let (provider, model, api_key) =
        crate::managers::meetings::llm_call::resolve_provider(&settings)?;
    let messages: Vec<StreamMessage> = messages
        .into_iter()
        .map(|m| StreamMessage::new(&m.role, m.content))
        .collect();
    let cancel = Arc::new(Notify::new());
    CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(request_id.clone(), cancel.clone());

    let splitter = Arc::new(Mutex::new(SentenceSplitter::new()));
    let index = Arc::new(Mutex::new(0u32));
    let emit_piece = {
        let app = app.clone();
        let request_id = request_id.clone();
        let index = index.clone();
        Arc::new(move |text: String| {
            let mut i = index.lock().unwrap_or_else(|e| e.into_inner());
            let _ = app.emit(
                EVENT,
                serde_json::json!({"kind": "sentence", "request_id": request_id, "index": *i, "text": text}),
            );
            *i += 1;
        })
    };
    let on_delta = {
        let splitter = splitter.clone();
        let emit_piece = emit_piece.clone();
        move |delta: &str| {
            let pieces = splitter
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(delta);
            for piece in pieces {
                emit_piece(piece);
            }
        }
    };

    // Erfassungsbereich: der Aufruf bucht seinen Verbrauch, und das Gespräch
    // zeigt Token und Kosten. Gelesen wird innerhalb des Bereichs, auch nach
    // dem Abbruch (dann ist nur der Verbrauch bis dahin gebucht, falls überhaupt).
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
        // Abgebrochen: das Restliche wird verworfen, die Sprachausgabe ist schon gestoppt.
        None => Ok(String::new()),
        Some(Ok(full)) => {
            let rest = splitter.lock().unwrap_or_else(|e| e.into_inner()).finish();
            if let Some(rest) = rest {
                emit_piece(rest);
            }
            let _ = app.emit(
                EVENT,
                serde_json::json!({
                    "kind": "done",
                    "request_id": request_id,
                    "usage": usage_payload(&captured),
                }),
            );
            Ok(full)
        }
        Some(Err(e)) => {
            // Auch ein fehlgeschlagener Aufruf kann schon Token verbraucht haben.
            let _ = app.emit(
                EVENT,
                serde_json::json!({
                    "kind": "failed",
                    "request_id": request_id,
                    "usage": (!captured.is_empty()).then(|| usage_payload(&captured)),
                }),
            );
            Err(e)
        }
    }
}

/// Bricht eine laufende Antwort ab (Unterbrechen durch den Sprecher).
#[tauri::command]
#[specta::specta]
pub fn voice_call_cancel(request_id: String) -> bool {
    let registered = CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&request_id)
        .cloned();
    match registered {
        Some(n) => {
            n.notify_one();
            true
        }
        None => {
            let mut early = EARLY_CANCELS.lock().unwrap_or_else(|e| e.into_inner());
            // Begrenzen: ein Abbruch ohne Anfrage darf nicht ewig liegen bleiben.
            if early.len() < 64 {
                early.insert(request_id);
            }
            false
        }
    }
}

/// Ein Häppchen mit der eingestellten Stimme zu WAV (Base64).
#[tauri::command]
#[specta::specta]
pub async fn voice_call_synthesize(app: AppHandle, text: String) -> Result<String, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let wav = tts.synthesize_wav(&text, Some(&cancel)).await?;
    Ok(B64.encode(wav))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_pcm16() {
        let bytes: Vec<u8> = [0i16, 16384, -32768]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let v = pcm16_b64_to_f32(&B64.encode(bytes)).unwrap();
        assert_eq!(v, vec![0.0, 0.5, -1.0]);
    }

    #[test]
    fn usage_payload_sums_calls() {
        let call = |p, c, id| CapturedCall {
            purpose: "chat".into(),
            provider_id: "openai".into(),
            local: false,
            connection_label: String::new(),
            model_id: "m".into(),
            model_label: "m".into(),
            prompt_tokens: p,
            completion_tokens: c,
            duration_ms: 1,
            ok: true,
            usage_event_id: id,
        };
        let v = usage_payload(&[call(10, 2, Some(7)), call(5, 1, None)]);
        assert_eq!(v["prompt_tokens"], 15);
        assert_eq!(v["completion_tokens"], 3);
        assert_eq!(v["usage_event_ids"], serde_json::json!([7]));
        assert_eq!(v["provider_id"], "openai");
        assert_eq!(usage_payload(&[])["prompt_tokens"], 0);
    }

    #[test]
    fn rejects_odd_length() {
        assert!(pcm16_b64_to_f32(&B64.encode([1u8, 2, 3])).is_err());
    }
}
