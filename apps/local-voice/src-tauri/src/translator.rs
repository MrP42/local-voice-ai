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
         text. Tokens like ⟦0⟧ stand for speaker names and sound tags: copy \
         every such token unchanged, exactly once, at the matching place.\n\n{text}"
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
        .ok_or_else(|| {
            "Kein Post-Processing-Provider konfiguriert (Einstellungen → Post Process)".to_string()
        })?;
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        return Err(format!(
            "Für '{}' ist kein Modell eingetragen (Einstellungen → Nachbearbeitung → Modell). Ganz lokal geht es mit dem Anbieter 'Ollama (lokal)' oder 'vLLM (lokal)'.",
            provider.label
        ));
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

// ---- Blockweise Übersetzung mit Fortschritt und geschützten Marken ----------

/// Zielgröße eines Übersetzungsblocks bei einem LOKALEN Modell (Zeichen). Ganze
/// Zeilen, nie mitten in einer Sprecherzeile getrennt. Kleine Modelle verlieren
/// bei langen Texten den Faden, ausserdem laufen sie ohne Mengenbegrenzung.
pub const BLOCK_CHARS_LOCAL: usize = 2500;
/// Bei Cloud-, API- und Abo-Modellen (Kontext ab 200 000 Token) geht der Text in
/// EINER Anfrage: viele kleine kosten Zeit und Kontingent (Anfragen je Minute und
/// Stunde) und bringen nichts. Die Grenze ist nur ein Sicherheitsnetz fuer sehr
/// lange Texte (die Antwort ist so lang wie der Text).
pub const BLOCK_CHARS_CLOUD: usize = 100_000;

/// Laeuft das aktive Sprachmodell auf diesem Rechner (App-Modell, Ollama, vLLM,
/// jede Adresse auf localhost)? Alles andere ist ein Cloud-, API- oder Abo-Modell.
pub fn active_model_is_local(settings: &AppSettings) -> bool {
    let Some(provider) = settings.active_post_process_provider() else {
        return false;
    };
    if crate::managers::llm::is_local(provider) || matches!(provider.id.as_str(), "ollama" | "vllm")
    {
        return true;
    }
    let url = provider.base_url.to_ascii_lowercase();
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(&url)
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    matches!(
        host,
        "localhost" | "127.0.0.1" | "[::1]" | "::1" | "0.0.0.0"
    )
}

/// Blockgroesse fuer die Uebersetzung: lokal klein, sonst eine Anfrage.
pub fn translation_block_chars(settings: &AppSettings) -> usize {
    if active_model_is_local(settings) {
        BLOCK_CHARS_LOCAL
    } else {
        BLOCK_CHARS_CLOUD
    }
}

/// Vorübergehende Fehler, bei denen ein zweiter Versuch Sinn hat: Modell
/// ausgelastet, Mengenbegrenzung, kurzer Ausfall.
pub fn is_transient_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    [
        "at capacity",
        "overloaded",
        "rate limit",
        "rate_limit",
        "429",
        "503",
        "try again",
        "temporarily",
    ]
    .iter()
    .any(|m| e.contains(m))
}

/// Abbruch der laufenden Übersetzung (`tts_transform_cancel`).
pub static TRANSLATE_CANCEL: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Fehlercode, wenn der Nutzer abgebrochen hat.
pub const CANCELLED: &str = "transform_cancelled";

/// Text in Blöcke ganzer Zeilen bis etwa `max` Zeichen. Leerzeilen bleiben
/// erhalten; zusammengesetzt mit `"\n"` ergibt sich wieder der Ausgangstext.
pub fn split_blocks(text: &str, max: usize) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut size = 0usize;
    for line in text.split('\n') {
        let len = line.chars().count() + 1;
        if !current.is_empty() && size + len > max {
            blocks.push(current.join("\n"));
            current.clear();
            size = 0;
        }
        current.push(line);
        size += len;
    }
    blocks.push(current.join("\n"));
    blocks
}

/// Sprechermarken `<Name>`/`<Name:Stil>` und Tags `[…]` (1–60 Zeichen, ohne
/// Zeilenumbruch) — dieselbe Regel wie die Sprechererkennung beim Vorlesen.
fn marker_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < bytes.len() {
        let (start, c) = bytes[i];
        let close = match c {
            '<' => '>',
            '[' => ']',
            _ => {
                i += 1;
                continue;
            }
        };
        let mut j = i + 1;
        let mut found = None;
        while j < bytes.len() && j - i <= 61 {
            let d = bytes[j].1;
            if d == close {
                found = Some(j);
                break;
            }
            if d == '\n' || d == c {
                break;
            }
            j += 1;
        }
        match found {
            Some(end) if end > i + 1 => {
                let end_byte = bytes[end].0 + close.len_utf8();
                spans.push((start, end_byte));
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    spans
}

/// Text mit Platzhaltern `⟦n⟧` statt der Marken und die Marken selbst.
pub fn mask_markers(text: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut tokens = Vec::new();
    let mut last = 0;
    for (s, e) in marker_spans(text) {
        out.push_str(&text[last..s]);
        out.push_str(&format!("⟦{}⟧", tokens.len()));
        tokens.push(text[s..e].to_string());
        last = e;
    }
    out.push_str(&text[last..]);
    (out, tokens)
}

/// Platzhalter zurück in Marken. `None`, wenn das Modell einen Platzhalter
/// verloren, verdoppelt oder erfunden hat — dann stimmt die Zuordnung nicht.
pub fn unmask_markers(text: &str, tokens: &[String]) -> Option<String> {
    let mut out = text.to_string();
    for (n, token) in tokens.iter().enumerate() {
        let placeholder = format!("⟦{n}⟧");
        if out.matches(&placeholder).count() != 1 {
            return None;
        }
        out = out.replacen(&placeholder, token, 1);
    }
    (!out.contains('⟦')).then_some(out)
}

/// Text in Marken und Zwischenstücke zerlegen (Rückfall, wenn das Modell die
/// Platzhalter nicht hält: dann wird nur zwischen den Marken übersetzt).
fn split_at_markers(text: &str) -> Vec<(bool, String)> {
    let mut parts = Vec::new();
    let mut last = 0;
    for (s, e) in marker_spans(text) {
        if s > last {
            parts.push((false, text[last..s].to_string()));
        }
        parts.push((true, text[s..e].to_string()));
        last = e;
    }
    if last < text.len() {
        parts.push((false, text[last..].to_string()));
    }
    parts
}

/// Führende und folgende Leerzeichen eines Stücks beim Übersetzen erhalten.
fn keep_padding(original: &str, translated: &str) -> String {
    let lead = &original[..original.len() - original.trim_start().len()];
    let trail = &original[original.trim_end().len()..];
    format!("{lead}{}{trail}", translated.trim())
}

/// Einen Block übersetzen: mit Platzhaltern, bei Verlust einmal neu, danach
/// Stück für Stück zwischen den Marken. `translate` ist der Modellaufruf.
pub async fn translate_block<F, Fut>(block: &str, mut translate: F) -> Result<String, String>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    if block.trim().is_empty() {
        return Ok(block.to_string());
    }
    let (masked, tokens) = mask_markers(block);
    if tokens.is_empty() {
        return Ok(keep_padding(block, &translate(masked).await?));
    }
    for _ in 0..2 {
        let reply = translate(masked.clone()).await?;
        if let Some(text) = unmask_markers(&reply, &tokens) {
            return Ok(keep_padding(block, &text));
        }
        log::warn!("Übersetzung: Platzhalter verloren, neuer Versuch");
    }
    log::warn!("Übersetzung: Platzhalter erneut verloren, übersetze zwischen den Marken");
    let mut out = String::new();
    for (is_marker, part) in split_at_markers(block) {
        if is_marker || !part.chars().any(char::is_alphabetic) {
            out.push_str(&part);
        } else {
            out.push_str(&keep_padding(
                &part,
                &translate(part.trim().to_string()).await?,
            ));
        }
    }
    Ok(out)
}

/// Ganzer Text, blockweise, mit Fortschritt (`done`, `total`) und Abbruch.
pub async fn translate_blocks(
    settings: &AppSettings,
    text: &str,
    target_lang: &str,
    cpu_only: bool,
    progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Result<String, String> {
    use std::sync::atomic::Ordering;
    TRANSLATE_CANCEL.store(false, Ordering::SeqCst);
    let blocks = split_blocks(text, translation_block_chars(settings));
    let total = blocks.len();
    progress(0, total);
    let mut out = Vec::with_capacity(total);
    for (i, block) in blocks.iter().enumerate() {
        if TRANSLATE_CANCEL.load(Ordering::SeqCst) {
            return Err(CANCELLED.to_string());
        }
        out.push(
            translate_block(block, |chunk| async move {
                // Ausgelastetes Modell oder Mengenbegrenzung: zweimal mit Pause neu.
                let mut attempt = 0;
                loop {
                    match translate_on(settings, &chunk, target_lang, cpu_only).await {
                        Err(e) if attempt < 2 && is_transient_error(&e) => {
                            log::warn!("Übersetzung: vorübergehender Fehler ({e}), neuer Versuch");
                            let pause = if attempt == 0 { 3 } else { 8 };
                            for _ in 0..pause * 4 {
                                if TRANSLATE_CANCEL.load(Ordering::SeqCst) {
                                    return Err(CANCELLED.to_string());
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                            }
                            attempt += 1;
                        }
                        other => return other,
                    }
                }
            })
            .await?,
        );
        progress(i + 1, total);
    }
    Ok(out.join("\n"))
}

#[cfg(test)]
mod block_tests {
    use super::*;

    fn settings_with(provider: &str, base_url: &str) -> AppSettings {
        let mut s = crate::settings::get_default_settings();
        if let Some(p) = s
            .post_process_providers
            .iter_mut()
            .find(|p| p.id == provider)
        {
            p.base_url = base_url.to_string();
        }
        s.post_process_provider_id = provider.to_string();
        s
    }

    #[test]
    fn cloud_models_get_one_request_and_local_ones_small_blocks() {
        let cloud = settings_with("openai", "https://api.openai.com/v1");
        assert!(!active_model_is_local(&cloud));
        assert_eq!(translation_block_chars(&cloud), BLOCK_CHARS_CLOUD);
        // Vier Seiten Text (~12 000 Zeichen): bei Cloud EIN Block, lokal mehrere.
        let text = "Ein Satz im Text. ".repeat(700);
        assert_eq!(
            split_blocks(&text, translation_block_chars(&cloud)).len(),
            1
        );
        let local = settings_with("ollama", "http://localhost:11434/v1");
        assert!(active_model_is_local(&local));
        assert_eq!(translation_block_chars(&local), BLOCK_CHARS_LOCAL);
    }

    #[test]
    fn a_remote_model_behind_a_custom_address_is_not_local() {
        let set_url = |s: &mut AppSettings, url: &str| {
            if let Some(p) = s
                .post_process_providers
                .iter_mut()
                .find(|p| p.id == "custom")
            {
                p.base_url = url.to_string();
            }
        };
        let mut s = crate::settings::get_default_settings();
        s.post_process_provider_id = "custom".into();
        set_url(&mut s, "https://llm.example.com/v1");
        assert!(!active_model_is_local(&s));
        set_url(&mut s, "http://127.0.0.1:8000/v1");
        assert!(active_model_is_local(&s));
    }

    #[test]
    fn busy_or_rate_limited_models_are_retried_other_errors_are_not() {
        assert!(is_transient_error(
            "cli_failed: {\"message\":\"Selected model is at capacity. Please try again\"}"
        ));
        assert!(is_transient_error(
            "Übersetzung fehlgeschlagen: 429 Too Many Requests"
        ));
        assert!(!is_transient_error(
            "cli_model_not_in_plan: Dieses Modell nimmt der Server nicht an"
        ));
        assert!(!is_transient_error("Kein Modell eingetragen"));
    }

    #[test]
    fn blocks_keep_whole_lines_and_rejoin_to_the_original() {
        let text = "<A> eins zwei\n\n<B> drei vier\n<A> fünf";
        let blocks = split_blocks(text, 16);
        assert!(blocks.len() > 1, "{blocks:?}");
        assert_eq!(blocks.join("\n"), text);
        assert!(blocks.iter().all(|b| !b.starts_with(' ')));
    }

    #[test]
    fn markers_and_tags_become_placeholders_and_come_back() {
        let text = "<Erzählerin> Hallo [long pause] Welt <Erzähler:ruhig> Ende";
        let (masked, tokens) = mask_markers(text);
        assert_eq!(masked, "⟦0⟧ Hallo ⟦1⟧ Welt ⟦2⟧ Ende");
        assert_eq!(tokens, ["<Erzählerin>", "[long pause]", "<Erzähler:ruhig>"]);
        assert_eq!(
            unmask_markers("⟦0⟧ Hello ⟦1⟧ world ⟦2⟧ end", &tokens).as_deref(),
            Some("<Erzählerin> Hello [long pause] world <Erzähler:ruhig> end")
        );
    }

    #[test]
    fn a_lost_or_doubled_placeholder_is_detected() {
        let tokens = vec!["<A>".to_string(), "<B>".to_string()];
        assert_eq!(unmask_markers("⟦0⟧ Hello", &tokens), None);
        assert_eq!(unmask_markers("⟦0⟧ ⟦0⟧ ⟦1⟧", &tokens), None);
        assert_eq!(unmask_markers("⟦0⟧ ⟦1⟧ ⟦2⟧", &tokens), None, "erfunden");
    }

    #[test]
    fn no_marker_in_plain_angle_text() {
        assert!(mask_markers("<>").1.is_empty());
        assert!(mask_markers("<zeile\nweiter>").1.is_empty());
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn speakers_are_never_translated() {
        let out = block_on(translate_block(
            "<Erzählerin> Guten Tag.",
            |s: String| async move {
                // Ein Modell, das brav die Platzhalter haelt.
                Ok(s.replace("Guten Tag.", "Good day."))
            },
        ))
        .unwrap();
        assert_eq!(out, "<Erzählerin> Good day.");
    }

    #[test]
    fn when_the_model_drops_placeholders_only_the_text_between_markers_is_translated() {
        let calls = std::cell::RefCell::new(Vec::new());
        let out = block_on(translate_block(
            "<Erzählerin> Guten Tag. <Erzähler> Hallo.",
            |s: String| {
                calls.borrow_mut().push(s.clone());
                async move {
                    Ok(if s.contains('⟦') {
                        "<Female narrator> Good day. <Male narrator> Hello.".to_string()
                    } else {
                        s.replace("Guten Tag.", "Good day.")
                            .replace("Hallo.", "Hello.")
                    })
                }
            },
        ))
        .unwrap();
        assert_eq!(out, "<Erzählerin> Good day. <Erzähler> Hello.");
        assert_eq!(calls.borrow().len(), 4, "2 Versuche + 2 Stücke");
    }
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
        assert!(err.contains("kein Modell eingetragen"), "war: {err}");
        assert!(
            err.contains("Einstellungen → Nachbearbeitung"),
            "Fehlermeldung nennt den Ort zum Nachtragen: {err}"
        );
        assert!(
            err.contains("Ollama"),
            "Fehlermeldung nennt den lokalen Weg: {err}"
        );
        // Der Anbieter steht mit seiner Beschriftung da, nicht mit der internen
        // Id — 'Custom' findet der Nutzer in der Liste, 'custom' nicht.
        assert!(err.contains("'Custom'"), "war: {err}");
    }
}
