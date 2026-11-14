//! Bildanfrage (Folien) an einen OpenAI-kompatiblen Server mit Projektor (lva#604, Teilpaket 3).
//!
//! Früher `llm_client::post_image_prompt`. Der Aufbau des Bodies (`image_request_body`) und die
//! Hilfen (`image_mime`, Grenzen) bleiben in `llm_client` (kein Netz). Neu gegenüber früher:
//! die Freigabe wird gegen das gebundene [`Ziel`] eingelöst, die Route läuft ohne Redirect
//! (lokal zusätzlich ohne Proxy), und ein Fehlerstatus trägt **nie** den Antworttext des
//! Servers (früher die ersten 300 Zeichen).

use std::path::Path;

use super::http::{client, sendefehler};
use crate::llm_client::{
    image_mime, image_request_body, ChatCompletionResponse, ImageReply, IMAGE_MAX_BYTES,
    IMAGE_TIMEOUT,
};
use crate::llm_versand::fehler::VersandFehler;
use crate::llm_versand::pruefer::Versandfreigabe;
use crate::llm_versand::quellen::Quellen;
use crate::llm_versand::ziel::Ziel;

/// Schickt `image` samt `prompt` an `<basis>/chat/completions` des gebundenen Ziels (die Basis
/// endet auf `/v1`). Fehlertexte tragen einen Code als Präfix: `image_unreadable`,
/// `image_unsupported`, `image_empty`, `image_too_large`, `request_failed`, `bad_response`.
pub(in crate::llm_versand) async fn senden(
    freigabe: Versandfreigabe,
    ziel: &Ziel,
    quellen: &Quellen,
    image: &Path,
    prompt: &str,
    max_tokens: u32,
) -> Result<ImageReply, String> {
    freigabe
        .einloesen(ziel, quellen)
        .map_err(|fehler| fehler.to_string())?;
    let basis = ziel
        .basis_url()
        .ok_or_else(|| "request_failed: kein HTTP-Ziel".to_string())?;
    let mime =
        image_mime(image).ok_or_else(|| "image_unsupported: kein JPEG/PNG/WebP".to_string())?;
    let bytes = std::fs::read(image).map_err(|e| format!("image_unreadable: {e}"))?;
    if bytes.is_empty() {
        return Err("image_empty: Bilddatei ist leer".to_string());
    }
    if bytes.len() > IMAGE_MAX_BYTES {
        return Err(format!("image_too_large: {} Byte", bytes.len()));
    }
    let body = image_request_body(ziel.modell(), prompt, mime, &bytes, max_tokens);
    drop(bytes);
    let client = client(ziel, None, Some(IMAGE_TIMEOUT))
        .map_err(|_| "request_failed: Client nicht aufbaubar".to_string())?;
    let response = client
        .post(format!("{}/chat/completions", basis.trim_end_matches('/')))
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            format!(
                "request_failed: {}",
                match sendefehler(&e) {
                    VersandFehler::Zeitueberschreitung => "Zeitlimit überschritten",
                    _ => "keine Verbindung",
                }
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        // Der Antworttext des Servers wird nicht gelesen und nicht weitergegeben.
        return Err(format!("request_failed: HTTP {status}"));
    }
    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|_| "bad_response: Antwort nicht lesbar".to_string())?;
    let first = completion.choices.first();
    Ok(ImageReply {
        content: first
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default(),
        truncated: first
            .and_then(|c| c.finish_reason.as_deref())
            .is_some_and(|r| r == "length"),
    })
}
