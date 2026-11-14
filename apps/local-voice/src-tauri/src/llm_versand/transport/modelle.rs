//! Modellliste eines OpenAI-kompatiblen Anbieters (lva#604, Teilpaket 3).
//!
//! Früher `llm_client::fetch_models` (nur der HTTP-Zweig; lokaler Anbieter und CLI antworten
//! ohne Netz und bleiben in der Fassade). Verwaltung, kein Inhalt: es geht kein Text einer
//! Besprechung hinaus. Trotzdem hier, damit **jede** Netzoperation des Modellpfads den
//! Transport und seine Regeln (kein Redirect, lokal ohne Proxy) nutzt.

use super::http::{client, sendefehler};
use crate::llm_client::build_headers;
use crate::llm_versand::fehler::VersandFehler;
use crate::llm_versand::pruefer::Versandfreigabe;
use crate::llm_versand::quellen::Quellen;
use crate::llm_versand::ziel::Ziel;
use crate::settings::PostProcessProvider;

/// `GET <basis>/models`; liefert die Modell-Kennungen.
pub(in crate::llm_versand) async fn liste(
    freigabe: Versandfreigabe,
    ziel: &Ziel,
    quellen: &Quellen,
    anbieter: &PostProcessProvider,
    api_key: &str,
) -> Result<Vec<String>, String> {
    freigabe
        .einloesen(ziel, quellen)
        .map_err(|fehler| fehler.to_string())?;
    let basis = ziel
        .basis_url()
        .ok_or_else(|| "Failed to fetch models: kein HTTP-Ziel".to_string())?;
    let url = format!("{}/models", basis.trim_end_matches('/'));
    log::debug!("Fetching models from: {}", ziel.adresse_fuer_log());
    let headers = build_headers(anbieter, api_key)?;
    let client =
        client(ziel, Some(headers), None).map_err(|_| "Failed to build HTTP client".to_string())?;

    let response = client.get(&url).send().await.map_err(|e| {
        format!(
            "Failed to fetch models: {}",
            match sendefehler(&e) {
                VersandFehler::Zeitueberschreitung => "Zeitlimit überschritten",
                _ => "keine Verbindung",
            }
        )
    })?;
    let status = response.status();
    if !status.is_success() {
        // Der Antworttext des Anbieters wird nicht weitergegeben.
        return Err(format!("Model list request failed ({status})"));
    }
    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Failed to parse response".to_string())?;

    let mut models = Vec::new();
    // OpenAI-Format: { data: [ { id: "..." }, ... ] }
    if let Some(data) = parsed.get("data").and_then(|d| d.as_array()) {
        for entry in data {
            if let Some(id) = entry.get("id").and_then(|i| i.as_str()) {
                models.push(id.to_string());
            } else if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
                models.push(name.to_string());
            }
        }
    }
    // Array-Format: [ "model1", "model2", ... ]
    else if let Some(array) = parsed.as_array() {
        for entry in array {
            if let Some(model) = entry.as_str() {
                models.push(model.to_string());
            }
        }
    }
    Ok(models)
}
