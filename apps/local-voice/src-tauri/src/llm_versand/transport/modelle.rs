//! Modellliste eines OpenAI-kompatiblen Anbieters (lva#604, Teilpaket 3).
//!
//! Früher `llm_client::fetch_models` (nur der HTTP-Zweig; lokaler Anbieter und CLI antworten
//! ohne Netz und bleiben in der Fassade). Verwaltung, kein Inhalt: es geht kein Text einer
//! Besprechung hinaus. Trotzdem hier, damit **jede** Netzoperation des Modellpfads den
//! Transport und seine Regeln (kein Redirect, lokal ohne Proxy) nutzt.

use super::http::{client, netzmeldung};
use crate::llm_client::build_headers;
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
    let client = client(ziel, Some(headers), None)
        .await
        .map_err(|_| "HTTP-Client für die Modellliste nicht aufbaubar.".to_string())?;

    let response = client.get(&url).send().await.map_err(netzmeldung)?;
    antwort_lesen(response).await
}

/// Fehlerkennung des Anbieters, nie dessen freier Text: Ein Freitext oder eine beliebige
/// Kennung kann den Schluessel enthalten (auch zerlegt, umgeordnet oder kodiert), das laesst
/// sich nicht verlaesslich schwaerzen (Codex-Review lva#654, Runden 3 bis 6). Darum kommt nur
/// eine Kennung aus der festen Liste [`BEKANNTE_KENNUNGEN`] durch, gesucht in `error.code`,
/// `error.type` und `code`; das Frontend zeigt sie als „Fehlerkennung des Anbieters“. Die
/// Ausgabe stammt damit immer aus dieser Liste, nie aus der Antwort.
fn anbieter_meldung(koerper: &[u8]) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_slice(koerper).ok()?;
    let fehler = parsed.get("error");
    let kandidaten = [
        fehler.and_then(|e| e.get("code")),
        fehler.and_then(|e| e.get("type")),
        parsed.get("code"),
    ];
    // `&'static str` aus der Liste: die Ausleihe von `parsed` endet hier.
    let bekannt: Option<&'static str> = kandidaten
        .into_iter()
        .flatten()
        .filter_map(|wert| wert.as_str())
        .find_map(|kennung| BEKANNTE_KENNUNGEN.iter().copied().find(|k| *k == kennung));
    bekannt.map(str::to_string)
}

/// Fehlerkennungen von OpenAI-kompatiblen Anbietern und Anthropic, die beim Abruf der
/// Modellliste vorkommen und dem Nutzer weiterhelfen (Schluessel, Region, Kontingent).
const BEKANNTE_KENNUNGEN: &[&str] = &[
    "invalid_api_key",
    "invalid_request_error",
    "unsupported_country_region_territory",
    "insufficient_quota",
    "rate_limit_exceeded",
    "invalid_organization",
    "invalid_project",
    "model_not_found",
    "authentication_error",
    "permission_error",
    "not_found_error",
    "rate_limit_error",
    "api_error",
    "overloaded_error",
    "billing_error",
];

/// Antwort auswerten; Fehler tragen einen stabilen Status und hoechstens eine Fehlerkennung.
async fn antwort_lesen(mut response: reqwest::Response) -> Result<Vec<String>, String> {
    let status = response.status();
    if !status.is_success() {
        let mut koerper = Vec::new();
        while koerper.len() < 8192 {
            match response.chunk().await {
                Ok(Some(stueck)) => {
                    let n = stueck.len().min(8192 - koerper.len());
                    koerper.extend_from_slice(&stueck[..n]);
                }
                // Auch bei unlesbarem Koerper bleibt der HTTP-Status erhalten.
                Ok(None) | Err(_) => break,
            }
        }
        let code = format!("http_status:{}", status.as_u16());
        return Err(match anbieter_meldung(&koerper) {
            Some(meldung) => format!("{code}: {meldung}"),
            None => code,
        });
    }
    let parsed: serde_json::Value = response.json().await.map_err(netzmeldung)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_versand::transport::http::client_mit_weg;
    use crate::netz::proxy::tests::{antwort, gesehen, testserver};
    use crate::netz::proxy::Weg;
    use std::time::Duration;

    #[tokio::test]
    async fn modellliste_meldet_die_proxyanmeldung_statt_rohtext() {
        let (proxy, log) = testserver(antwort("407 Proxy Authentication Required", "")).await;
        let ziel = Ziel::http("custom", "http://example.invalid/v1", "m");
        let client = client_mit_weg(
            &ziel,
            None,
            Some(Duration::from_secs(5)),
            &Weg::Ueber(proxy),
        )
        .unwrap();
        let response = client
            .get("http://example.invalid/v1/models")
            .send()
            .await
            .unwrap();
        let meldung = antwort_lesen(response).await.unwrap_err();
        assert_eq!(meldung, "http_status:407");
        assert_eq!(
            gesehen(&log),
            ["GET http://example.invalid/v1/models HTTP/1.1"]
        );
    }

    async fn fehlerantwort(status: &str, koerper: &str) -> String {
        let roh = format!(
            "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{koerper}",
            koerper.len()
        );
        let (proxy, _) = testserver(roh).await;
        let ziel = Ziel::http("custom", "http://example.invalid/v1", "m");
        let client = client_mit_weg(
            &ziel,
            None,
            Some(Duration::from_secs(5)),
            &Weg::Ueber(proxy),
        )
        .unwrap();
        let response = client
            .get("http://example.invalid/v1/models")
            .send()
            .await
            .unwrap();
        antwort_lesen(response).await.unwrap_err()
    }

    #[tokio::test]
    async fn modellliste_zeigt_die_fehlerkennung_nie_den_freitext() {
        let koerper = concat!(
            r#"{"error":{"message":"Incorrect API key provided: "#,
            "sk-proj-GEHEIM1234567890abcd. You can find your API key at ",
            r#"https://platform.openai.com/account/api-keys.","#,
            r#""type":"invalid_request_error","code":"invalid_api_key"}}"#
        );
        let meldung = fehlerantwort("401 Unauthorized", koerper).await;
        assert_eq!(meldung, "http_status:401: invalid_api_key");
    }

    #[tokio::test]
    async fn modellliste_uebernimmt_keinen_html_fehlertext() {
        let meldung = fehlerantwort(
            "403 Forbidden",
            "<html><title>Cache Access Denied</title></html>",
        )
        .await;
        assert_eq!(meldung, "http_status:403");
    }

    #[test]
    fn freitext_wird_nie_uebernommen_auch_nicht_zerlegt() {
        for text in [
            "Rejected value: QwE rTy UiO pAs DfG h",
            "Invalid credential: QwErTyUiOpAsDfGh",
            "Invalid key ending in fGh",
            "Invalid key AIzaSyABC***QRST",
        ] {
            for koerper in [
                serde_json::json!({ "error": { "message": text } }),
                serde_json::json!({ "message": text }),
                serde_json::json!({ "error": text }),
            ] {
                assert_eq!(anbieter_meldung(koerper.to_string().as_bytes()), None);
            }
        }
    }

    #[test]
    fn nur_kennungen_aus_der_festen_liste() {
        for (koerper, erwartet) in [
            (
                r#"{"error":{"code":"unsupported_country_region_territory"}}"#,
                Some("unsupported_country_region_territory"),
            ),
            (
                r#"{"error":{"type":"permission_error"}}"#,
                Some("permission_error"),
            ),
            (
                r#"{"code":"insufficient_quota"}"#,
                Some("insufficient_quota"),
            ),
            // code unbekannt: type als Ersatz
            (
                r#"{"error":{"code":"Key sk-proj-GEHEIM","type":"authentication_error"}}"#,
                Some("authentication_error"),
            ),
            (r#"{"error":{"code":401}}"#, None),
            (r#"{"error":{"code":"some_other_code"}}"#, None),
            (r#"{}"#, None),
            (r#"not json"#, None),
        ] {
            assert_eq!(
                anbieter_meldung(koerper.as_bytes()).as_deref(),
                erwartet,
                "{koerper}"
            );
        }
    }

    #[test]
    fn schluessel_in_der_kennung_kommt_nie_durch() {
        // Codex R4 bis R6: zerlegt, kurz, umgedreht, kleingeschrieben.
        for koerper in [
            r#"{"error":{"code":"qwe_rty_uio_pas_dfg_h"}}"#,
            r#"{"code":"abc"}"#,
            r#"{"error":{"code":"abc_________"}}"#,
            r#"{"error":{"code":"lkjihgfedcba"}}"#,
            r#"{"error":{"type":"sk_proj_geheim"}}"#,
        ] {
            assert_eq!(anbieter_meldung(koerper.as_bytes()), None, "{koerper}");
        }
        // Jede Ausgabe ist ein Eintrag der festen Liste.
        let ausgabe = anbieter_meldung(br#"{"error":{"code":"invalid_api_key"}}"#).unwrap();
        assert!(BEKANNTE_KENNUNGEN.contains(&ausgabe.as_str()));
    }
}
