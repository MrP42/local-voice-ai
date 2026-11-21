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
    antwort_lesen(response, api_key).await
}

/// Fehlerkennung des Anbieters (z. B. `invalid_api_key`, `unsupported_country_region_territory`),
/// nie dessen freier Text: Ein Freitext kann den Schluessel in beliebiger Zerlegung enthalten,
/// das laesst sich nicht verlaesslich schwaerzen (Codex-Review lva#654, Runde 3). Zugelassen
/// sind nur Kennungen aus Kleinbuchstaben und `_` (3 bis 64 Zeichen) aus `error.code`,
/// `error.type` oder `code`; das Frontend zeigt sie als „Fehlerkennung des Anbieters“.
// Der Zwischenwert `kennung` beendet die Ausleihe von `parsed` vor dessen Ende (E0597).
#[allow(clippy::let_and_return)]
fn anbieter_meldung(koerper: &[u8], api_key: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_slice(koerper).ok()?;
    let fehler = parsed.get("error");
    let kandidaten = [
        fehler.and_then(|e| e.get("code")),
        fehler.and_then(|e| e.get("type")),
        parsed.get("code"),
    ];
    let key = api_key.trim();
    // Kein echter Schluessel ist so kurz; dann lieber gar keine Kennung (Codex R4).
    if !key.is_empty() && key.chars().count() < MIN_SCHLUESSEL {
        return None;
    }
    let key = nur_buchstaben(key);
    // Zu wenige Buchstaben fuer einen Vergleich (`abc_________`, Codex R5): keine Kennung.
    if !api_key.trim().is_empty() && key.len() < MIN_BUCHSTABEN {
        return None;
    }
    let kennung = kandidaten
        .into_iter()
        .flatten()
        .filter_map(|wert| wert.as_str())
        .find(|kennung| ist_kennung(kennung) && !teilt_stueck_mit(kennung, &key))
        .map(str::to_string);
    kennung
}

/// Nur `[a-z_]`, 3 bis 64 Zeichen: keine Ziffern, keine Grossbuchstaben, kein Leerraum.
fn ist_kennung(text: &str) -> bool {
    (3..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

/// Kuerzeste Schluessellaenge, bei der eine Kennung ueberhaupt uebernommen wird.
const MIN_SCHLUESSEL: usize = 12;

/// Mindestzahl an Buchstaben im Schluessel, damit der Stueckvergleich etwas aussagt.
const MIN_BUCHSTABEN: usize = 8;

/// Nur die Buchstaben, klein geschrieben: `qwe_rty` und `QwE-rTy1` werden beide `qwerty`.
fn nur_buchstaben(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Teilt die Kennung ein Stueck von 4 Buchstaben mit dem Schluessel? Verglichen wird ohne
/// Trennzeichen, Ziffern und Gross-/Kleinschreibung, damit ein in `_`-Stuecke zerlegter
/// Schluessel (`qwe_rty_uio`) ebenso auffaellt wie ein zusammenhaengender (Codex R4).
fn teilt_stueck_mit(kennung: &str, key_buchstaben: &str) -> bool {
    let kennung = nur_buchstaben(kennung);
    let zeichen: Vec<char> = key_buchstaben.chars().collect();
    zeichen
        .windows(4)
        .any(|f| kennung.contains(f.iter().collect::<String>().as_str()))
}

/// Antwort auswerten; Fehler tragen einen stabilen Status und hoechstens eine Fehlerkennung.
async fn antwort_lesen(
    mut response: reqwest::Response,
    api_key: &str,
) -> Result<Vec<String>, String> {
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
        return Err(match anbieter_meldung(&koerper, api_key) {
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
        let meldung = antwort_lesen(response, "").await.unwrap_err();
        assert_eq!(meldung, "http_status:407");
        assert_eq!(
            gesehen(&log),
            ["GET http://example.invalid/v1/models HTTP/1.1"]
        );
    }

    async fn fehlerantwort(status: &str, koerper: &str, api_key: &str) -> String {
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
        antwort_lesen(response, api_key).await.unwrap_err()
    }

    #[tokio::test]
    async fn modellliste_zeigt_die_fehlerkennung_nie_den_freitext() {
        let key = "sk-proj-GEHEIM1234567890abcd";
        let koerper = concat!(
            r#"{"error":{"message":"Incorrect API key provided: "#,
            "sk-proj-GEHEIM1234567890abcd. You can find your API key at ",
            r#"https://platform.openai.com/account/api-keys.","#,
            r#""type":"invalid_request_error","code":"invalid_api_key"}}"#
        );
        let meldung = fehlerantwort("401 Unauthorized", koerper, key).await;
        assert_eq!(meldung, "http_status:401: invalid_api_key");
    }

    #[tokio::test]
    async fn modellliste_uebernimmt_keinen_html_fehlertext() {
        let meldung = fehlerantwort(
            "403 Forbidden",
            "<html><title>Cache Access Denied</title></html>",
            "",
        )
        .await;
        assert_eq!(meldung, "http_status:403");
    }

    #[test]
    fn freitext_wird_nie_uebernommen_auch_nicht_zerlegt() {
        let key = "QwErTyUiOpAsDfGh";
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
                assert_eq!(anbieter_meldung(koerper.to_string().as_bytes(), key), None);
            }
        }
    }

    #[test]
    fn nur_kennungen_aus_kleinbuchstaben_und_unterstrich() {
        let key = "sk-proj-GEHEIM1234567890abcd";
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
            // code fehlt oder ist keine Kennung: type als Ersatz
            (
                r#"{"error":{"code":"Key sk-proj-GEHEIM","type":"authentication_error"}}"#,
                Some("authentication_error"),
            ),
            (r#"{"error":{"code":401}}"#, None),
            (r#"{"error":{"code":"GEHEIM1234"}}"#, None),
            (r#"{"error":{"code":"ab"}}"#, None),
            (r#"{"error":{"code":"with space"}}"#, None),
            (r#"{}"#, None),
            (r#"not json"#, None),
        ] {
            assert_eq!(
                anbieter_meldung(koerper.as_bytes(), key).as_deref(),
                erwartet,
                "{koerper}"
            );
        }
    }

    #[test]
    fn zerlegter_oder_kurzer_schluessel_kommt_nie_als_kennung_durch() {
        // Codex R4: Trennzeichen zerlegen den Schluessel in Stuecke unter 4 Zeichen.
        for (key, koerper) in [
            (
                "qwertyuiopasdfgh",
                r#"{"error":{"code":"qwe_rty_uio_pas_dfg_h"}}"#,
            ),
            (
                "QwErTyUiOpAsDfGh",
                r#"{"error":{"code":"qwe_rty_uio_pas_dfg_h"}}"#,
            ),
            ("Q1w2E3r4T5y6U7i8", r#"{"error":{"type":"qwe_rty_uio"}}"#),
            ("abc", r#"{"code":"abc"}"#),
            ("abc", r#"{"code":"invalid_api_key"}"#),
            ("abc_________", r#"{"error":{"code":"abc_________"}}"#),
            ("12345678901234ab", r#"{"code":"invalid_api_key"}"#),
        ] {
            assert_eq!(
                anbieter_meldung(koerper.as_bytes(), key),
                None,
                "{key} {koerper}"
            );
        }
    }

    #[test]
    fn kennung_mit_einem_schluesselstueck_faellt_weg() {
        // Ein (unrealistischer) Schluessel aus Kleinbuchstaben: die Kennung enthaelt ein Stueck davon.
        let key = "invalid_secret_value";
        assert_eq!(
            anbieter_meldung(br#"{"error":{"code":"invalid_api_key"}}"#, key),
            None
        );
        assert_eq!(
            anbieter_meldung(br#"{"error":{"code":"insufficient_quota"}}"#, key).as_deref(),
            Some("insufficient_quota")
        );
    }
}
