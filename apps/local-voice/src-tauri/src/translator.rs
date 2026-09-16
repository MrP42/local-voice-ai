//! Textübersetzung für die Audio-Übersetzung (TP3).
//!
//! Bewusst über die vorhandene Post-Process-Provider-Infrastruktur: der
//! Nutzer konfiguriert Provider/Modell/Key an einer Stelle, und „Custom"
//! zeigt per Default auf das lokale Ollama (http://localhost:11434/v1) —
//! damit bleibt auch die Übersetzung vollständig lokal möglich.

use crate::settings::AppSettings;

/// Ein Prompt, der nur die Übersetzung zurückverlangt — keine Erklärungen,
/// keine Anführungszeichen, Ton und Namen bleiben erhalten.
pub fn translation_prompt(target_lang: &str, text: &str) -> String {
    format!(
        "Translate the following text into {target_lang}. Reply with ONLY the \
         translation - no explanations, no quotation marks around it. Keep \
         names and numbers unchanged and match the tone of the original. \
         Preserve the layout of the original exactly: the same line breaks, \
         the same blank lines between paragraphs, one output paragraph for \
         one input paragraph. Never merge paragraphs into a single block of \
         text.\n\n{text}"
    )
}

/// Übersetzt über den aktiven Post-Process-Provider.
pub async fn translate(
    settings: &AppSettings,
    text: &str,
    target_lang: &str,
) -> Result<String, String> {
    translate_on(settings, text, target_lang, false).await
}

/// Wie [`translate`], aber mit der Wahl, die GPU freizulassen.
///
/// `cpu_only` greift nur bei einem lokalen Ollama: dessen nativer Endpunkt
/// `/api/chat` nimmt `num_gpu: 0` entgegen, der OpenAI-kompatible Pfad kennt
/// keine Geräteauswahl. Bei entfernten Anbietern stellt sich die Frage nicht,
/// bei vLLM gibt es die Möglichkeit nicht — dort läuft alles wie bisher.
///
/// Der Grund: der Fish-Speech-Server belegt rund 17 GB Grafikspeicher. Ein
/// Übersetzungsmodell daneben bringt beide zum Straucheln.
pub async fn translate_on(
    settings: &AppSettings,
    text: &str,
    target_lang: &str,
    cpu_only: bool,
) -> Result<String, String> {
    let provider = settings
        .active_post_process_provider()
        .cloned()
        .ok_or_else(|| crate::llm_client::MODEL_SETUP_REQUIRED.to_string())?;
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        return Err(crate::llm_client::MODEL_SETUP_REQUIRED.to_string());
    }
    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    let prompt = translation_prompt(target_lang, text);
    // Übersetzen ist Umformen, kein Nachdenken: Reasoning aus, wo steuerbar
    // (gleiche Provider-Sonderfälle wie beim Post-Processing).
    let (reasoning_effort, reasoning) = match provider.id.as_str() {
        "custom" => (Some("none".to_string()), None),
        "openrouter" => (
            None,
            Some(crate::llm_client::ReasoningConfig {
                effort: Some("none".to_string()),
                exclude: Some(true),
            }),
        ),
        _ => (None, None),
    };
    // Auf der CPU nur ueber den nativen Weg — und wenn der nicht antwortet,
    // lieber uebersetzen als scheitern: dann eben auf der GPU.
    let native = cpu_only
        .then(|| crate::llm_client::ollama_native_url(&provider.base_url))
        .flatten();
    if let Some(url) = native {
        match crate::llm_client::send_ollama_native(
            crate::managers::usage::Purpose::Translation,
            &provider,
            &url,
            &model,
            prompt.clone(),
            true,
        )
        .await
        {
            Ok(Some(content)) => {
                let trimmed = content.trim().to_string();
                if !trimmed.is_empty() {
                    return Ok(trimmed);
                }
                log::warn!("Ollama (CPU) lieferte leere Uebersetzung — versuche den ueblichen Weg");
            }
            Ok(None) => log::warn!("Ollama (CPU) ohne Inhalt — versuche den ueblichen Weg"),
            Err(e) => {
                log::warn!("Ollama (CPU) nicht erreichbar ({e}) — versuche den ueblichen Weg")
            }
        }
    }

    let outcome = match crate::llm_client::send_chat_completion(
        crate::managers::usage::Purpose::Translation,
        &provider,
        api_key,
        &model,
        prompt,
        reasoning_effort,
        reasoning,
    )
    .await
    {
        Ok(Some(content)) => {
            let trimmed = content.trim().to_string();
            if trimmed.is_empty() {
                Err("Übersetzung kam leer zurück".into())
            } else {
                Ok(trimmed)
            }
        }
        Ok(None) => Err("Übersetzungsantwort ohne Inhalt".into()),
        Err(e) => Err(format!("Übersetzung fehlgeschlagen: {e}")),
    };
    // Ein lokales Ollama-Modell sofort wieder entladen — der Speicher gehört
    // dem Fish-Speech-Server. Der kompatible Pfad kennt kein keep_alive,
    // deshalb der Nachschuss; bei entfernten Anbietern läuft er ins Leere
    // (ollama_native_url liefert dort None). Auch nach einem FEHLER: gerade
    // ein abgebrochener Lauf lässt das Modell sonst geladen stehen.
    crate::llm_client::ollama_unload(&provider.base_url, &model).await;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Der native Endpunkt ist der einzige, der die GPU abwaehlen laesst.
    #[test]
    fn der_native_ollama_endpunkt_wird_richtig_abgeleitet() {
        use crate::llm_client::ollama_native_url;
        assert_eq!(
            ollama_native_url("http://localhost:11434/v1").as_deref(),
            Some("http://localhost:11434/api/chat")
        );
        assert_eq!(
            ollama_native_url("http://127.0.0.1:11434/v1/").as_deref(),
            Some("http://127.0.0.1:11434/api/chat")
        );
        // Entfernte Anbieter: die Frage stellt sich nicht.
        assert_eq!(ollama_native_url("https://api.openai.com/v1"), None);
        assert_eq!(ollama_native_url("https://api.groq.com/openai/v1"), None);
    }

    #[test]
    fn prompt_names_the_target_language_and_forbids_chatter() {
        let p = translation_prompt("German", "Hello world");
        assert!(p.contains("into German"));
        assert!(p.contains("ONLY the"));
        assert!(p.ends_with("Hello world"));
    }

    /// Absätze sind der Grund, warum ein vorgelesener Text lesbar bleibt.
    /// Ohne diese Ansage lieferten Modelle eine einzige Textwand zurück.
    #[test]
    fn prompt_verlangt_die_absaetze_des_originals() {
        let p = translation_prompt("English", "Erster Absatz.\n\nZweiter Absatz.");
        assert!(p.contains("blank lines between paragraphs"));
        assert!(p.contains("Never merge paragraphs"));
        // Der Text selbst geht mit seinen Umbrüchen hinein, nicht geglättet.
        assert!(p.ends_with("Erster Absatz.\n\nZweiter Absatz."));
    }

    /// Mock eines OpenAI-kompatiblen /chat/completions-Endpunkts.
    async fn spawn_llm_mock(reply: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    let mut read = 0usize;
                    loop {
                        let n = sock.read(&mut buf[read..]).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        read += n;
                        let text = String::from_utf8_lossy(&buf[..read]).to_lowercase();
                        if let Some(header_end) = text.find("\r\n\r\n") {
                            let content_length = text
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length: "))
                                .and_then(|v| v.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            if read >= header_end + 4 + content_length {
                                let body = format!(
                                    r#"{{"choices":[{{"message":{{"role":"assistant","content":"{reply}"}}}}]}}"#
                                );
                                let head = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                                    body.len()
                                );
                                let _ = sock.write_all(head.as_bytes()).await;
                                let _ = sock.write_all(body.as_bytes()).await;
                                let _ = sock.shutdown().await;
                                break;
                            }
                        }
                    }
                });
            }
        });
        port
    }

    fn settings_with_mock_provider(port: u16) -> crate::settings::AppSettings {
        let mut settings = get_default_settings();
        settings.post_process_provider_id = "custom".into();
        if let Some(custom) = settings.post_process_provider_mut("custom") {
            custom.base_url = format!("http://127.0.0.1:{port}/v1");
        }
        settings
            .post_process_models
            .insert("custom".into(), "test-model".into());
        settings
    }

    #[tokio::test]
    async fn translate_returns_the_models_reply() {
        let port = spawn_llm_mock("Hallo Welt").await;
        let settings = settings_with_mock_provider(port);
        let out = translate(&settings, "Hello world", "German").await.unwrap();
        assert_eq!(out, "Hallo Welt");
    }

    #[tokio::test]
    async fn translate_without_configured_model_fails_with_guidance() {
        let mut settings = get_default_settings();
        settings.post_process_provider_id = "custom".into();
        settings
            .post_process_models
            .insert("custom".into(), "".into());
        let err = translate(&settings, "Hello", "German").await.unwrap_err();
        assert_eq!(err, crate::llm_client::MODEL_SETUP_REQUIRED);
    }
}
