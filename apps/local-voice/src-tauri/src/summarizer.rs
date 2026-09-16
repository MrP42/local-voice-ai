//! LLM-Zusammenfassung von Texten/Dokumenten vor dem Vorlesen.
//!
//! Nutzt dieselbe Provider-Infrastruktur wie Post-Processing und Übersetzung
//! (lokal via Ollama, Subscription/API via OpenAI, Anthropic, OpenRouter, …).
//! Lange Quellen werden in Blöcke geteilt, blockweise zusammengefasst und die
//! Teilergebnisse in einer zweiten Stufe verdichtet (map-reduce).

use serde::Deserialize;
use specta::Type;

use crate::settings::AppSettings;

/// Blockgröße in Zeichen — konservativ gewählt, damit auch kleine lokale
/// Modelle (8k Kontext) jeden Block samt Prompt verarbeiten können.
const CHUNK_CHARS: usize = 16_000;

#[derive(Debug, Clone, Deserialize, Type)]
pub struct SummaryOptions {
    /// "kurz" (~150 Wörter) | "mittel" (~400) | "lang" (~900)
    pub length: String,
    /// "ueberblick" | "ausgewogen" | "detailliert"
    pub detail: String,
    /// "allgemein" | "fachpublikum" | "management" | "einfache_sprache"
    pub audience: String,
}

fn length_words(length: &str) -> u32 {
    match length {
        "kurz" => 150,
        "lang" => 900,
        _ => 400,
    }
}

fn detail_instruction(detail: &str) -> &'static str {
    match detail {
        "ueberblick" => "Focus only on the core message and main conclusions.",
        "detailliert" => {
            "Cover all significant points, arguments, numbers and examples, not just the headlines."
        }
        _ => "Balance the big picture with the most important supporting details.",
    }
}

fn audience_instruction(audience: &str) -> &'static str {
    match audience {
        "fachpublikum" => "Write for an expert audience; keep technical terms precise.",
        "management" => {
            "Write for decision makers: lead with outcomes, implications and recommendations."
        }
        "einfache_sprache" => {
            "Write in simple, easy-to-understand language with short sentences (plain language)."
        }
        _ => "Write for a general audience without assuming prior knowledge.",
    }
}

/// Prompt für einen Quelltext(-Block). Antwortsprache = Sprache der Quelle,
/// damit die Zusammenfassung in derselben Stimme vorgelesen werden kann.
pub fn summary_prompt(opts: &SummaryOptions, text: &str) -> String {
    format!(
        "Summarize the following text in the SAME language as the text itself{}. \
         Target length: about {} words. {} {} Write flowing prose that works \
         well when read aloud (no bullet points, no headings, no meta commentary). \
         Reply with ONLY the summary.\n\n{text}",
        language_clause(text),
        length_words(&opts.length),
        detail_instruction(&opts.detail),
        audience_instruction(&opts.audience),
    )
}

/// Kleine Modelle antworten auf einen englischen Prompt gern auf Englisch,
/// egal was "same language" sagt. Deshalb wird die erkannte Sprache der
/// Quelle beim Namen genannt — das hält auch Qwen 3 4B in der Spur.
fn language_clause(text: &str) -> String {
    match crate::managers::tts::lang::detect_language(text) {
        Some("de") => " (that is: write in German)".into(),
        Some("en") => " (that is: write in English)".into(),
        Some("fr") => " (that is: write in French)".into(),
        Some("es") => " (that is: write in Spanish)".into(),
        Some("it") => " (that is: write in Italian)".into(),
        _ => String::new(),
    }
}

/// Prompt der Aufbereitung: Text fürs Vorlesen säubern, ohne Inhalt zu
/// verlieren. Seitenzahlen, Kopf-/Fußzeilen, Fußnotenziffern, Silbentrennung
/// am Zeilenende und harte Umbrüche mitten im Satz sind Artefakte aus PDF
/// und Zwischenablage — sie würden sonst mitgelesen.
pub fn tidy_prompt(text: &str) -> String {
    format!(
        "Clean up the following text so it can be read aloud, in the SAME language \
         as the text itself{}. Rules: keep EVERY sentence and all information, in the \
         original order; do not summarize, shorten, translate or add anything. Remove \
         only reading artefacts: page numbers, running headers and footers, footnote \
         markers and reference numbers, table-of-contents lines, stray list numbering \
         and bullet symbols, line-break hyphenation, and hard line breaks inside a \
         sentence. Join broken lines into proper paragraphs; keep one blank line \
         between paragraphs. Keep speaker markers like <Name> and tags in square \
         brackets like [calm] exactly where they are. Reply with ONLY the cleaned \
         text, no commentary.\n\n{text}",
        language_clause(text),
    )
}

/// Text fürs Vorlesen aufbereiten; lange Texte blockweise, die Blöcke werden
/// in Reihenfolge wieder zusammengesetzt (kein Reduce — nichts darf fehlen).
pub async fn tidy(settings: &AppSettings, text: &str) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Kein Text zum Aufbereiten".into());
    }
    let chunks = chunk_text(trimmed, CHUNK_CHARS);
    let mut parts = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        parts.push(ask_llm(settings, tidy_prompt(chunk)).await?.trim().to_string());
    }
    let out = parts.join("\n\n");
    // Ein Modell, das kuerzt, ist hier ein Fehler: die Aufbereitung darf
    // Artefakte nehmen, keinen Inhalt. Unter 60 % der Zeichen ist Verdacht.
    if out.chars().count() * 10 < trimmed.chars().count() * 6 {
        return Err("Die Aufbereitung hat zu viel Text verworfen – Ergebnis nicht übernommen.".into());
    }
    Ok(out)
}

/// Prompt der Reduce-Stufe: Teilzusammenfassungen zu einer verdichten.
pub fn combine_prompt(opts: &SummaryOptions, partials: &[String]) -> String {
    format!(
        "The following are partial summaries of consecutive sections of one \
         document, in order. Merge them into ONE coherent summary in the same \
         language, about {} words. {} {} Flowing prose for read-aloud, no lists, \
         no meta commentary. Reply with ONLY the summary.\n\n{}",
        length_words(&opts.length),
        detail_instruction(&opts.detail),
        audience_instruction(&opts.audience),
        partials.join("\n\n---\n\n"),
    )
}

/// Text an Absatz-/Satzgrenzen in Blöcke von höchstens `max_chars` teilen.
pub fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for paragraph in text.split_inclusive("\n") {
        if current.chars().count() + paragraph.chars().count() > max_chars
            && !current.trim().is_empty()
        {
            chunks.push(current.trim().to_string());
            current = String::new();
        }
        // Ein einzelner Absatz über dem Limit wird hart an Zeichengrenzen geteilt.
        if paragraph.chars().count() > max_chars {
            let mut piece = String::new();
            for c in paragraph.chars() {
                piece.push(c);
                if piece.chars().count() >= max_chars {
                    chunks.push(piece.trim().to_string());
                    piece = String::new();
                }
            }
            current.push_str(&piece);
        } else {
            current.push_str(paragraph);
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }
    chunks
}

async fn ask_llm(settings: &AppSettings, prompt: String) -> Result<String, String> {
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
    match crate::llm_client::send_chat_completion(
        crate::managers::usage::Purpose::Summary,
        &provider,
        api_key,
        &model,
        prompt,
        None,
        None,
    )
        .await
    {
        Ok(Some(content)) => {
            let trimmed = content.trim().to_string();
            if trimmed.is_empty() {
                Err("Zusammenfassung kam leer zurück".into())
            } else {
                Ok(trimmed)
            }
        }
        Ok(None) => Err("Antwort ohne Inhalt".into()),
        Err(e) => Err(format!("Zusammenfassung fehlgeschlagen: {e}")),
    }
}

/// Quelltext zusammenfassen; lange Quellen laufen zweistufig (map-reduce).
pub async fn summarize(
    settings: &AppSettings,
    text: &str,
    opts: &SummaryOptions,
) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Kein Text zum Zusammenfassen".into());
    }
    let chunks = chunk_text(trimmed, CHUNK_CHARS);
    if chunks.len() <= 1 {
        return ask_llm(settings, summary_prompt(opts, trimmed)).await;
    }
    log::info!("summarize: {} Blöcke (map-reduce)", chunks.len());
    let mut partials = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        partials.push(ask_llm(settings, summary_prompt(opts, chunk)).await?);
    }
    ask_llm(settings, combine_prompt(opts, &partials)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_summary_model_requests_setup_before_inference() {
        let mut settings = crate::settings::get_default_settings();
        settings.post_process_models.clear();
        assert_eq!(
            ask_llm(&settings, "Test".into()).await.unwrap_err(),
            crate::llm_client::MODEL_SETUP_REQUIRED
        );
        settings.post_process_provider_id = "removed".into();
        assert_eq!(
            ask_llm(&settings, "Test".into()).await.unwrap_err(),
            crate::llm_client::MODEL_SETUP_REQUIRED
        );
    }

    fn opts() -> SummaryOptions {
        SummaryOptions {
            length: "kurz".into(),
            detail: "ueberblick".into(),
            audience: "management".into(),
        }
    }

    #[test]
    fn the_prompt_carries_every_option() {
        let p = summary_prompt(&opts(), "Quelltext hier.");
        assert!(p.contains("about 150 words"));
        assert!(p.contains("core message"));
        assert!(p.contains("decision makers"));
        assert!(p.contains("SAME language"));
        assert!(p.ends_with("Quelltext hier."));
    }

    #[test]
    fn the_prompt_names_the_detected_language() {
        let p = summary_prompt(&opts(), "Die drei Feen und die Wolke voller Wünsche über den Hügeln.");
        assert!(p.contains("write in German"));
        let e = summary_prompt(&opts(), "It was a quiet evening and the flowers had closed.");
        assert!(e.contains("write in English"));
        assert!(!summary_prompt(&opts(), "Luminara").contains("write in"));
    }

    #[test]
    fn the_tidy_prompt_keeps_content_and_markers() {
        let p = tidy_prompt("<Anna> [calm] Seite 3 Text.");
        assert!(p.contains("keep EVERY sentence"));
        assert!(p.contains("<Name>"));
        assert!(p.contains("[calm]"));
        assert!(p.ends_with("<Anna> [calm] Seite 3 Text."));
    }

    #[test]
    fn unknown_option_values_fall_back_to_the_middle_ground() {
        let p = summary_prompt(
            &SummaryOptions {
                length: "xxl".into(),
                detail: "?".into(),
                audience: "?".into(),
            },
            "T",
        );
        assert!(p.contains("about 400 words"));
        assert!(p.contains("Balance the big picture"));
        assert!(p.contains("general audience"));
    }

    #[test]
    fn long_texts_are_chunked_at_paragraph_boundaries() {
        let paragraph = "Ein Absatz mit etwas Inhalt.\n";
        let text = paragraph.repeat(1_000); // ~29k Zeichen
        let chunks = chunk_text(&text, 16_000);
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| c.chars().count() <= 16_000));
        assert!(chunks.iter().all(|c| c.contains("Ein Absatz")));

        assert_eq!(chunk_text("kurz", 16_000).len(), 1);
        let monster = "x".repeat(40_000);
        let hard = chunk_text(&monster, 16_000);
        assert_eq!(hard.len(), 3, "absatzlose Monster werden hart geteilt");
    }
}
