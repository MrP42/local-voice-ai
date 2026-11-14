//! Natives Ollama (`/api/chat`, `/api/generate`) für Übersetzung und Auto-Tagging
//! (lva#604, Teilpaket 3).
//!
//! Früher `llm_client::send_ollama_native` und `ollama_unload`. Die Adresse des Endpunkts steht
//! im gebundenen [`Ziel`] (`Ziel::basis_url` ist hier die vollständige Endpunkt-URL). Neu
//! gegenüber früher: Redirect-Verbot, lokale Route ohne Proxy, Cloud als Cloud (die alte
//! Prüfung `contains("localhost")` ersetzt nichts mehr; das `Ziel` leitet die Art aus dem Host
//! ab), und keine Antworttexte in Fehlern.

use super::http::{client, sendefehler, statusfehler};
use super::{Fehlschlag, Versuch};
use crate::llm_versand::fehler::VersandFehler;
use crate::llm_versand::pruefer::Versandfreigabe;
use crate::llm_versand::quellen::Quellen;
use crate::llm_versand::ziel::Ziel;
use crate::managers::usage::TokenUsage;

/// Eine Anfrage über Ollamas nativen Endpunkt, wahlweise ohne GPU (`num_gpu: 0`); `keep_alive:
/// 0` entlädt das Modell direkt nach der Antwort.
pub(in crate::llm_versand) async fn nativ(
    freigabe: Versandfreigabe,
    ziel: &Ziel,
    quellen: &Quellen,
    prompt: String,
    cpu_only: bool,
) -> Versuch<Option<String>> {
    freigabe.einloesen(ziel, quellen)?;
    let url = ziel.basis_url().ok_or(VersandFehler::Transport)?;
    let mut options = serde_json::json!({});
    if cpu_only {
        options["num_gpu"] = serde_json::json!(0);
    }
    let body = serde_json::json!({
        "model": ziel.modell(),
        "messages": [{ "role": "user", "content": prompt }],
        "stream": false,
        "keep_alive": 0,
        "options": options,
    });
    let client = client(ziel, None, None)?;
    let response = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| Fehlschlag::Fehler(sendefehler(&e)))?;
    if !response.status().is_success() {
        return Err(Fehlschlag::Fehler(statusfehler(response).await));
    }
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|_| Fehlschlag::Fehler(VersandFehler::Protokoll))?;
    // Ollamas eigene Zähler: `prompt_eval_count` und `eval_count`.
    let count = |key: &str| value.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
    let tokens = TokenUsage {
        prompt_tokens: count("prompt_eval_count"),
        completion_tokens: count("eval_count"),
    };
    Ok((
        value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string()),
        tokens,
    ))
}

/// Ein bei Ollama geladenes Modell sofort entladen (best effort). Scheitern ist kein Fehler des
/// Aufrufers: dann läuft Ollamas eigene Frist ab; nur ein Protokolleintrag, ohne Antworttext.
pub(in crate::llm_versand) async fn entladen(
    freigabe: Versandfreigabe,
    ziel: &Ziel,
    quellen: &Quellen,
) {
    if freigabe.einloesen(ziel, quellen).is_err() {
        return;
    }
    let Some(url) = ziel.basis_url() else {
        return;
    };
    let Ok(client) = client(ziel, None, None) else {
        return;
    };
    let body = serde_json::json!({ "model": ziel.modell(), "keep_alive": 0 });
    match client.post(url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            log::info!(
                "Ollama-Modell '{}' nach der Übersetzung entladen",
                ziel.modell()
            );
        }
        Ok(resp) => log::warn!("Ollama unload answered {}", resp.status()),
        Err(e) => log::warn!("Ollama unload failed ({:?})", sendefehler(&e)),
    }
}
