//! Token-Bedarf eines KI-Dokuments: Prognose vor dem Start, Ist nach dem Lauf.
//!
//! **Prognose.** Der echte Ablauf (`engine::run` bzw. `kurzbuch::run`) läuft
//! einmal trocken: statt des Modells antwortet [`simulated_answer`] mit einem
//! Platzhaltertext in der Länge, die der Prompt verlangt („about 300 words“).
//! Gezählt werden die tatsächlich gebauten Prompts samt Vorlagenrahmen,
//! Kapitelaufteilung, Verdichtungsrunden und Gliederung. Zwischenfassungen,
//! die schon im Cache des Buchs liegen, kosten nichts – der Cache wird dabei
//! nur gelesen, nie geschrieben. Zeichen werden in Token umgerechnet: beim
//! laufenden lokalen Server mit dessen Tokenizer an einer Stichprobe, sonst
//! mit einem Durchschnittswert.
//!
//! **Ist.** Der Lauf steht in einem Erfassungsbereich (`usage::with_capture`);
//! summiert wird, was die Anbieter in `usage` melden (OpenAI-kompatibel,
//! llama-server, Ollama, Anthropic über die OpenAI-Schnittstelle). Kosten
//! nach den Preisen der aktiven Modellkonfiguration.
//!
//! Im Log und in Fehlern stehen nur Zähler, nie Text.

use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::engine::{self, DigestCache, RunInput};
use super::kurzbuch;
use crate::managers::usage::{self, CapturedCall, NewUsageEvent, Purpose, TokenUsage};
use crate::settings::AppSettings;

/// Zeichen je Token ohne Messung. Deutsche und englische Prosa liegen bei
/// gängigen Tokenizern (Llama/Qwen, GPT, Claude) zwischen etwa 3,2 und 4,2.
pub const DEFAULT_CHARS_PER_TOKEN: f64 = 3.6;
/// Größe der Stichprobe für den Tokenizer des lokalen Servers.
const SAMPLE_CHARS: usize = 6_000;
/// Rahmen je Aufruf (Chat-Vorlage, Rollen), den der Prompttext nicht enthält.
const CALL_OVERHEAD_TOKENS: u64 = 12;
/// Länge eines Platzhalterworts samt Leerzeichen (Prosa: deutsch ~6–7, englisch ~5–6).
const FILLER_WORD: &str = "worte ";
/// Antwortlänge, wenn der Prompt keine verlangt.
const DEFAULT_ANSWER_WORDS: usize = 400;

/// Token-Zahlen eines Laufs, geschätzt oder gemessen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TokenFigures {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Modellaufrufe (ohne Cache-Treffer).
    pub calls: u32,
    /// Kosten in Mikro-Dollar; `None`, wenn für das Modell kein Preis hinterlegt ist.
    pub cost_micro: Option<i64>,
}

impl TokenFigures {
    pub fn total(&self) -> u32 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// Verbrauch eines KI-Dokuments: was vorher geschätzt und was gebraucht wurde.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct KiUsage {
    pub estimate: Option<TokenFigures>,
    pub actual: Option<TokenFigures>,
    /// Lief auf diesem Rechner (keine Kosten bei einem Anbieter).
    pub local: bool,
}

/// Prognose vor dem Start (`book_ai_estimate`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct BookAiEstimate {
    pub figures: TokenFigures,
    /// Läuft das aktive Modell lokal?
    pub local: bool,
    /// Wurde mit dem Tokenizer des Modells kalibriert (sonst Durchschnittswert)?
    pub measured: bool,
    /// Bereits zwischengespeicherte Arbeitsschritte, die nichts mehr kosten.
    pub cached_steps: u32,
    /// Modell, nach dessen Preis gerechnet wurde (ohne Anbieter).
    pub model: String,
}

/// Zielwortzahl aus „about 300 words“ (auch „About“). Erste Fundstelle.
fn requested_words(prompt: &str) -> Option<usize> {
    let lower = prompt.to_ascii_lowercase();
    let mut rest = lower.as_str();
    while let Some(i) = rest.find("about ") {
        let after = &rest[i + 6..];
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && after[digits.len()..].starts_with(" words") {
            return digits.parse().ok();
        }
        rest = after;
    }
    None
}

fn filler(words: usize) -> String {
    FILLER_WORD.repeat(words.max(1)).trim_end().to_string()
}

/// Platzhalterantwort in der Länge, die ein echtes Modell liefern würde.
pub fn simulated_answer(prompt: &str, ideas: usize) -> String {
    if prompt.starts_with("You plan a short summary") {
        // Gliederung: so viele Kernaussagen wie empfohlen, Teile ordnet
        // `parse_outline` selbst zu.
        let ideas: Vec<String> = (0..ideas.max(1))
            .map(|k| {
                format!(
                    "{{\"title\": \"Kernaussage {k}: {}\", \"parts\": []}}",
                    filler(12)
                )
            })
            .collect();
        return format!(
            "{{\"about\": \"{}\", \"audience\": [\"{a}\", \"{a}\", \"{a}\"], \"ideas\": [{}]}}",
            filler(45),
            ideas.join(", "),
            a = filler(5),
        );
    }
    if prompt.starts_with("The following are digests") {
        // Verdichtung: etwa halb so lang wie die Eingabe.
        let words = prompt.split_whitespace().count() / 2;
        return filler(words.max(60));
    }
    filler(requested_words(prompt).unwrap_or(DEFAULT_ANSWER_WORDS))
}

/// Zeichen in Token.
pub fn chars_to_tokens(chars: usize, chars_per_token: f64) -> u64 {
    if chars == 0 {
        return 0;
    }
    (chars as f64 / chars_per_token.max(1.0)).ceil() as u64
}

/// Aufrufe der Trockenfahrt (Zeichen je Richtung).
#[derive(Default)]
struct DryRun {
    calls: u32,
    prompt_chars: usize,
    answer_chars: usize,
}

/// Läuft den echten Ablauf ohne Modell und zählt Zeichen. `cache` wird nur
/// gelesen (siehe [`DigestCache::read_only`]).
pub async fn dry_run(
    input: RunInput<'_>,
    cache: &DigestCache,
    kurzbuch_run: bool,
) -> Result<(u32, usize, usize), String> {
    let words: usize = input
        .sections
        .iter()
        .map(|s| super::source::word_count(&s.text) as usize)
        .sum();
    let ideas = kurzbuch::suggested_ideas(words);
    let log = Mutex::new(DryRun::default());
    let ask = |prompt: String| {
        let answer = simulated_answer(&prompt, ideas);
        {
            let mut l = log.lock().unwrap_or_else(|e| e.into_inner());
            l.calls += 1;
            l.prompt_chars += prompt.chars().count();
            l.answer_chars += answer.chars().count();
        }
        std::future::ready(Ok::<String, String>(answer))
    };
    let cancel = AtomicBool::new(false);
    let progress = |_: usize, _: usize| {};
    if kurzbuch_run {
        kurzbuch::run(input, cache, ask, &progress, &cancel).await?;
    } else {
        engine::run(input, cache, ask, &progress, &cancel).await?;
    }
    let l = log.into_inner().unwrap_or_else(|e| e.into_inner());
    Ok((l.calls, l.prompt_chars, l.answer_chars))
}

/// Zeichen je Token für das aktive Modell. Misst nur, wenn der lokale Server
/// mit genau diesem Modell schon läuft – eine Prognose startet kein Modell.
pub async fn chars_per_token(settings: &AppSettings, sample: &str) -> (f64, bool) {
    let Some(provider) = settings.active_post_process_provider() else {
        return (DEFAULT_CHARS_PER_TOKEN, false);
    };
    if !crate::managers::llm::is_local(provider) {
        return (DEFAULT_CHARS_PER_TOKEN, false);
    }
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.is_empty() || crate::managers::llm::active_context_tokens(&model).is_none() {
        return (DEFAULT_CHARS_PER_TOKEN, false);
    }
    let sample: String = sample.chars().take(SAMPLE_CHARS).collect();
    let chars = sample.chars().count();
    if chars < 500 {
        return (DEFAULT_CHARS_PER_TOKEN, false);
    }
    let Some(root) = crate::managers::llm::local_server_root(&model).await else {
        return (DEFAULT_CHARS_PER_TOKEN, false);
    };
    match crate::managers::llm::tokenize_count(&root, &sample).await {
        Some(tokens) if tokens > 0 => ((chars as f64 / tokens as f64).clamp(1.5, 6.0), true),
        _ => (DEFAULT_CHARS_PER_TOKEN, false),
    }
}

/// Preise (USD je Million Token) des aktiven Modells, wenn es zum Anbieter passt.
fn prices(settings: &AppSettings) -> (Option<f64>, Option<f64>) {
    let Some(provider) = settings.active_post_process_provider() else {
        return (None, None);
    };
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    let e = NewUsageEvent::from_call(
        Purpose::Summary,
        provider,
        &model,
        settings.active_llm_model(),
        TokenUsage::default(),
        0,
        Ok(()),
    );
    (e.price_input_per_mtok, e.price_output_per_mtok)
}

/// Läuft das aktive Modell auf diesem Rechner?
pub fn active_is_local(settings: &AppSettings) -> bool {
    settings
        .active_post_process_provider()
        .is_some_and(usage::provider_is_local)
}

fn cost(input: u64, output: u64, price: (Option<f64>, Option<f64>), local: bool) -> Option<i64> {
    if local {
        return Some(0);
    }
    if price.0.is_none() && price.1.is_none() {
        return None;
    }
    Some(usage::cost_micro(
        TokenUsage {
            prompt_tokens: input,
            completion_tokens: output,
        },
        price.0,
        price.1,
    ))
}

fn clamp_u32(v: u64) -> u32 {
    v.min(u32::MAX as u64) as u32
}

/// Prognose aus einer Trockenfahrt.
pub fn figures_from_dry_run(
    calls: u32,
    prompt_chars: usize,
    answer_chars: usize,
    chars_per_token: f64,
    settings: &AppSettings,
) -> TokenFigures {
    let local = active_is_local(settings);
    let input =
        chars_to_tokens(prompt_chars, chars_per_token) + CALL_OVERHEAD_TOKENS * calls as u64;
    let output = chars_to_tokens(answer_chars, chars_per_token);
    TokenFigures {
        input_tokens: clamp_u32(input),
        output_tokens: clamp_u32(output),
        calls,
        cost_micro: cost(input, output, prices(settings), local),
    }
}

/// Liefen alle erfassten Aufrufe auf diesem Rechner?
pub fn calls_local(calls: &[CapturedCall]) -> bool {
    !calls.is_empty() && calls.iter().all(|c| c.local)
}

/// Ist-Werte aus den erfassten Aufrufen eines Laufs. Gescheiterte Aufrufe
/// sind mit null Token gebucht und zählen nicht als Aufruf.
pub fn figures_from_calls(calls: &[CapturedCall], settings: &AppSettings) -> TokenFigures {
    let input: u64 = calls.iter().map(|c| c.prompt_tokens).sum();
    let output: u64 = calls.iter().map(|c| c.completion_tokens).sum();
    let local = calls_local(calls);
    let ok = calls.iter().filter(|c| c.ok).count();
    TokenFigures {
        input_tokens: clamp_u32(input),
        output_tokens: clamp_u32(output),
        calls: ok.min(u32::MAX as usize) as u32,
        cost_micro: cost(input, output, prices(settings), local),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book_ai::source::Section;
    use crate::book_ai::templates::builtins;
    use std::future::Future;

    fn section(title: &str, words: usize) -> Section {
        Section {
            title: title.into(),
            text: "Wort ".repeat(words).trim().to_string(),
            first_page: Some(1),
            last_page: Some(2),
        }
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    #[test]
    fn reads_the_requested_length() {
        assert_eq!(requested_words("x. About 120 words. y"), Some(120));
        assert_eq!(
            requested_words("Target length: about 300 words.\n"),
            Some(300)
        );
        assert_eq!(requested_words("about fifteen minutes"), None);
        let a = simulated_answer("Write about 200 words of prose", 3);
        assert_eq!(a.split_whitespace().count(), 200);
    }

    #[test]
    fn the_simulated_outline_is_readable() {
        let a = simulated_answer("You plan a short summary of a book", 5);
        let o = kurzbuch::parse_outline(&a, 4, 3).expect("Gliederung");
        assert_eq!(o.ideas.len(), 5);
        assert_eq!(o.audience.len(), 3);
    }

    #[test]
    fn short_sources_cost_one_call() {
        let t = &builtins()[0];
        let s = [section("Eins", 500)];
        let (calls, pin, pout) = block_on(dry_run(
            RunInput {
                sections: &s,
                template: t,
                limit: 40_000,
                model_key: "m",
                language: "",
            },
            &DigestCache::read_only(None),
            false,
        ))
        .unwrap();
        assert_eq!(calls, 1);
        assert!(pin > 2_500, "Prompt enthält den Text: {pin}");
        assert!(pout > 0);
    }

    #[test]
    fn long_books_need_digests_and_the_kurzbuch_more_steps() {
        let t = &builtins()[0];
        let s: Vec<Section> = (0..12).map(|i| section(&format!("K{i}"), 4_000)).collect();
        let input = || RunInput {
            sections: &s,
            template: t,
            limit: 16_000,
            model_key: "m",
            language: "",
        };
        let (calls, _, _) =
            block_on(dry_run(input(), &DigestCache::read_only(None), false)).unwrap();
        assert!(calls > 12, "Zwischenfassungen + letzter Schritt: {calls}");
        let (kb_calls, _, _) =
            block_on(dry_run(input(), &DigestCache::read_only(None), true)).unwrap();
        // Gliederung, je Kernaussage ein Schritt und das Fazit kommen dazu.
        assert!(kb_calls >= calls + 3, "{kb_calls} vs {calls}");
    }

    #[test]
    fn cached_digests_cost_nothing_and_are_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let t = &builtins()[0];
        let s: Vec<Section> = (0..6).map(|i| section(&format!("K{i}"), 4_000)).collect();
        let input = || RunInput {
            sections: &s,
            template: t,
            limit: 16_000,
            model_key: "m",
            language: "",
        };
        let cache = DigestCache::read_only(Some(dir.path().to_path_buf()));
        let (cold, _, _) = block_on(dry_run(input(), &cache, false)).unwrap();
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "Trockenfahrt schreibt keinen Cache"
        );
        // Echte Zwischenfassungen ablegen, dann kostet nur noch der letzte Schritt.
        let writer = DigestCache {
            dir: Some(dir.path().to_path_buf()),
            read_only: false,
        };
        block_on(engine::run(
            input(),
            &writer,
            |_| async { Ok("Zwischenfassung".to_string()) },
            &|_, _| {},
            &AtomicBool::new(false),
        ))
        .unwrap();
        let (warm, _, _) = block_on(dry_run(input(), &cache, false)).unwrap();
        assert!(warm < cold, "{warm} < {cold}");
        assert_eq!(warm, 1);
    }

    #[test]
    fn converts_and_prices() {
        assert_eq!(chars_to_tokens(0, 3.6), 0);
        assert_eq!(chars_to_tokens(36, 3.6), 10);
        assert_eq!(
            cost(1_000_000, 0, (Some(3.0), Some(15.0)), false),
            Some(3_000_000)
        );
        assert_eq!(cost(1_000_000, 1_000, (None, None), false), None);
        assert_eq!(cost(5, 5, (Some(3.0), None), true), Some(0));
    }

    #[test]
    fn sums_captured_calls() {
        let call = |p: u64, c: u64| CapturedCall {
            purpose: "summary".into(),
            provider_id: "local".into(),
            local: true,
            connection_label: String::new(),
            model_id: "m".into(),
            model_label: "m".into(),
            prompt_tokens: p,
            completion_tokens: c,
            duration_ms: 1,
            ok: true,
            usage_event_id: None,
        };
        let mut failed = call(0, 0);
        failed.ok = false;
        let f = figures_from_calls(
            &[call(100, 20), call(50, 5), failed],
            &crate::settings::get_default_settings(),
        );
        assert_eq!((f.input_tokens, f.output_tokens, f.calls), (150, 25, 2));
        assert_eq!(f.cost_micro, Some(0));
        assert_eq!(f.total(), 175);
    }
}
