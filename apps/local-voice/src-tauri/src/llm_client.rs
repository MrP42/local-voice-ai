use crate::managers::usage::{self, Purpose, TokenUsage};
use crate::settings::PostProcessProvider;
use log::debug;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct JsonSchema {
    name: String,
    strict: bool,
    schema: Value,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
    json_schema: JsonSchema,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ReasoningConfig>,
    /// Nur lokal und nur fuer Denkmodelle: schaltet das Denken ab (P1g).
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_template_kwargs: Option<Value>,
    /// Nur lokal und nur fuer KI-Notizen und Follow-up-Mail: deterministisch (siehe
    /// `deterministic_sampling`).
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u32>,
}

/// Temperatur und Startwert fuer einen nicht-streamenden Aufruf: die KI-Notizen
/// (Erzeugen, Map/Reduce, Anweisung anwenden) und seit P1k das Protokoll samt
/// Vorlagenwahl laufen lokal deterministisch, wie der Chat seit P4g (A3: auch die
/// KI-Zusammenfuehrung von Transkript-Fassungen). Sonst sampelt das Modell mal ohne, mal mit
/// verworfener Quell-ID, und die Notizen-Eval (Soll ai_sourced >= 0,95)
/// besteht nur zufaellig. Entfernte Anbieter und alle anderen Zwecke bekommen
/// die Felder nicht: manche lehnen sie mit 400 ab (Denkmodelle).
fn deterministic_sampling(local: bool, purpose: Purpose) -> (Option<f32>, Option<u32>) {
    if local
        && matches!(
            purpose,
            Purpose::EnhancedNotes
                | Purpose::Followup
                | Purpose::Minutes
                | Purpose::TranscriptMerge
                | Purpose::TranscriptTranslation
        )
    {
        (Some(0.0), Some(CHAT_SEED))
    } else {
        (None, None)
    }
}

/// Denkt dieses lokale Modell vor der Antwort, sofern man es nicht abschaltet?
/// Qwen3 und Qwen3.5 (Katalog-Kennungen `llm-qwen3-...`, `llm-qwen3.5-...`)
/// tun das: ein Prompt "2+3" kostet 623 Token, und bei einem langen Prompt ist
/// der Kontext voll, bevor das JSON beginnt (P1e, Befund B3). Andere Modelle
/// (Gemma, Llama, ...) bekommen nichts.
pub fn is_thinking_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("qwen3")
}

/// `chat_template_kwargs` fuer einen nicht-streamenden Aufruf: nur beim
/// lokalen Server und nur fuer Denkmodelle. Entfernte Anbieter lehnen
/// unbekannte Felder teils mit 400 ab.
fn thinking_off_kwargs(local: bool, model: &str) -> Option<Value> {
    (local && is_thinking_model(model)).then(|| serde_json::json!({ "enable_thinking": false }))
}

/// Antworttext eines lokalen Servers ohne Denk-Reste: verirrt sich trotz
/// abgeschaltetem Denken ein `<think>...</think>` in die Antwort (aeltere
/// Chat-Vorlagen, `--reasoning-format none`), ist es sonst der Anfang eines
/// kaputten JSON. Entfernte Anbieter bleiben unveraendert.
fn clean_local_content(local: bool, content: Option<String>) -> Option<String> {
    match content {
        Some(text) if local && text.contains("think>") => {
            Some(crate::managers::meetings::chat::citations::strip_think(&text).trim_start().to_string())
        }
        other => other,
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    /// Token-Zaehler, wie OpenAI, llama-server, Ollama und OpenRouter sie
    /// liefern. Fehlt bei manchen Anbietern -- dann wird mit null gebucht.
    #[serde(default)]
    usage: Option<UsageBlock>,
}

#[derive(Debug, Deserialize, Default)]
struct UsageBlock {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

impl From<Option<UsageBlock>> for TokenUsage {
    fn from(block: Option<UsageBlock>) -> Self {
        let b = block.unwrap_or_default();
        TokenUsage {
            prompt_tokens: b.prompt_tokens,
            completion_tokens: b.completion_tokens,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
    /// `length`: der Server hat die Antwort abgebrochen (Kontext oder
    /// Ausgabegrenze voll). Fehlt bei manchen Anbietern.
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatMessageResponse {
    content: Option<String>,
}

/// Build headers for API requests based on provider type
fn build_headers(provider: &PostProcessProvider, api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();

    // Common headers
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://github.com/cjpais/Handy"),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Handy/1.0 (+https://github.com/cjpais/Handy)"),
    );
    headers.insert("X-Title", HeaderValue::from_static("Handy"));

    // Provider-specific auth headers
    if !api_key.is_empty() {
        if provider.id == "anthropic" {
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(api_key)
                    .map_err(|e| format!("Invalid API key header value: {}", e))?,
            );
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        } else {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", api_key))
                    .map_err(|e| format!("Invalid authorization header value: {}", e))?,
            );
        }
    }

    Ok(headers)
}

/// Create an HTTP client with provider-specific headers
fn create_client(provider: &PostProcessProvider, api_key: &str) -> Result<reqwest::Client, String> {
    let headers = build_headers(provider, api_key)?;
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// Send a chat completion request to an OpenAI-compatible API
/// Returns Ok(Some(content)) on success, Ok(None) if response has no content,
/// or Err on actual errors (HTTP, parsing, etc.)
pub async fn send_chat_completion(
    purpose: Purpose,
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    prompt: String,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
) -> Result<Option<String>, String> {
    send_chat_completion_with_schema(
        purpose,
        provider,
        api_key,
        model,
        prompt,
        None,
        None,
        reasoning_effort,
        reasoning,
    )
    .await
}

/// Antwort eines Chat-Aufrufs samt dem Hinweis, ob der Server sie abgeschnitten
/// hat (`finish_reason == "length"`). Ein abgeschnittenes JSON ist nie gueltig;
/// wer es weiss, kann den Prompt verkleinern statt denselben noch einmal zu
/// senden (P1i, Befund B11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatReply {
    pub content: Option<String>,
    pub truncated: bool,
}

/// Send a chat completion request with structured output support
/// When json_schema is provided, uses structured outputs mode
/// system_prompt is used as the system message when provided
/// reasoning_effort sets the OpenAI-style top-level field (e.g., "none", "low", "medium", "high")
/// reasoning sets the OpenRouter-style nested object (effort + exclude)
///
/// `purpose` sagt dem Verbrauchs-Ledger, wofuer der Aufruf war. Jeder
/// Aufruf wird gebucht -- auch ein gescheiterter, dann mit null Token.
#[allow(clippy::too_many_arguments)]
pub async fn send_chat_completion_with_schema(
    purpose: Purpose,
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<Value>,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
) -> Result<Option<String>, String> {
    send_chat_completion_checked(
        purpose,
        provider,
        api_key,
        model,
        user_content,
        system_prompt,
        json_schema,
        reasoning_effort,
        reasoning,
    )
    .await
    .map(|reply| reply.content)
}

/// Wie [`send_chat_completion_with_schema`], liefert aber auch, ob die Antwort
/// abgeschnitten wurde.
#[allow(clippy::too_many_arguments)]
pub async fn send_chat_completion_checked(
    purpose: Purpose,
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<Value>,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
) -> Result<ChatReply, String> {
    // Hartes Budget: die Verweigerung ist bewusst ungebucht -- es wurde ja
    // nichts verbraucht.
    // Regelwerk vor dem Budget: ein gesperrter Anbieter bekommt gar nichts.
    crate::managers::compliance::check_call(provider)?;
    usage::check_budget(provider, model)?;
    let started = std::time::Instant::now();
    let (result, tokens) = match send_inner(
        purpose,
        provider,
        api_key,
        model,
        user_content,
        system_prompt,
        json_schema,
        reasoning_effort,
        reasoning,
    )
    .await
    {
        Ok((reply, tokens)) => (Ok(reply), tokens),
        Err(e) => (Err(e), TokenUsage::default()),
    };
    let elapsed = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
    usage::record_call(
        purpose,
        provider,
        model,
        tokens,
        elapsed,
        result.as_ref().map(|_| ()).map_err(|e| e.clone()),
    )
    .await;
    result
}

/// Der eigentliche Aufruf; liefert Antwort und Token-Zaehler getrennt, damit
/// der Aufrufer oben beides buchen kann.
#[allow(clippy::too_many_arguments)]
async fn send_inner(
    purpose: Purpose,
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<Value>,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
) -> Result<(ChatReply, TokenUsage), String> {
    // Abo-Modelle (Claude Code, Codex) laufen ueber die CLI des Anbieters.
    if let Some(cli) = crate::managers::llm::cli::Cli::from_base_url(&provider.base_url) {
        return send_cli(cli, model, user_content, system_prompt, json_schema).await;
    }
    // Fuer den lokalen Anbieter ist die Adresse erst bekannt, wenn der
    // Server laeuft -- und der wird hier bei Bedarf gestartet.
    let resolved = crate::managers::llm::resolve_base_url(provider, model).await?;
    let local = crate::managers::llm::is_local(provider);
    let sampling = deterministic_sampling(local, purpose);
    let base_url = resolved.trim_end_matches('/');
    let url = format!("{}/chat/completions", base_url);

    debug!("Sending chat completion request to: {}", url);

    let client = create_client(provider, &api_key)?;

    // Build messages vector
    let mut messages = Vec::new();

    // Add system prompt if provided
    if let Some(system) = system_prompt {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: system,
        });
    }

    // Add user message
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: user_content,
    });

    // Build response_format if schema is provided
    let response_format = json_schema.map(|schema| ResponseFormat {
        format_type: "json_schema".to_string(),
        json_schema: JsonSchema {
            name: "transcription_output".to_string(),
            strict: true,
            schema,
        },
    });

    let request_body = ChatCompletionRequest {
        model: model.to_string(),
        messages,
        response_format,
        reasoning_effort,
        reasoning,
        chat_template_kwargs: thinking_off_kwargs(local, model),
        temperature: sampling.0,
        seed: sampling.1,
    };

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read error response".to_string());
        return Err(format!(
            "API request failed with status {}: {}",
            status, error_text
        ));
    }

    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse API response: {}", e))?;

    let tokens = TokenUsage::from(completion.usage);
    let first = completion.choices.first();
    let content = first.and_then(|choice| choice.message.content.clone());
    let truncated = first
        .and_then(|choice| choice.finish_reason.as_deref())
        .is_some_and(|reason| reason == "length");
    Ok((
        ChatReply {
            content: clean_local_content(local, content),
            truncated,
        },
        tokens,
    ))
}

/// Abo-Modell ueber die CLI. Ein Schema wandert in den Systemtext (die CLIs
/// kennen keine strukturierte Ausgabe); ein Codeblock um das JSON wird entfernt.
async fn send_cli(
    cli: crate::managers::llm::cli::Cli,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<Value>,
) -> Result<(ChatReply, TokenUsage), String> {
    use crate::managers::llm::cli;
    let binary = cli::locate(cli)
        .ok_or_else(|| "cli_missing: Die CLI des Anbieters ist nicht installiert".to_string())?;
    let wants_json = json_schema.is_some();
    let system = match json_schema {
        Some(schema) => Some(format!(
            "{}\n\nAntworte ausschliesslich mit einem JSON-Objekt nach diesem JSON-Schema, \
             ohne Erklaerung und ohne Codeblock:\n{}",
            system_prompt.unwrap_or_default(),
            schema
        )),
        None => system_prompt,
    };
    let model = model.to_string();
    let reply = tokio::task::spawn_blocking(move || {
        cli::call(cli, &binary, &model, system.as_deref(), &user_content)
    })
    .await
    .map_err(|e| format!("cli_failed: {e}"))??;
    let text = if wants_json {
        strip_code_fence(&reply.text)
    } else {
        reply.text
    };
    Ok((
        ChatReply {
            content: Some(text),
            truncated: false,
        },
        TokenUsage {
            prompt_tokens: reply.prompt_tokens,
            completion_tokens: reply.completion_tokens,
        },
    ))
}

/// ```json … ``` um eine JSON-Antwort entfernen.
fn strip_code_fence(text: &str) -> String {
    let t = text.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return t.to_string();
    };
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    rest.strip_suffix("```").unwrap_or(rest).trim().to_string()
}

/// Fetch available models from an OpenAI-compatible API
/// Returns a list of model IDs
pub async fn fetch_models(
    provider: &PostProcessProvider,
    api_key: String,
) -> Result<Vec<String>, String> {
    // Der lokale Anbieter listet, was auf der Platte liegt -- dafuer muss
    // kein Server laufen, und einer ohne Modell koennte es auch nicht.
    if crate::managers::llm::is_local(provider) {
        return Ok(crate::managers::llm::downloaded_model_ids());
    }
    if let Some(cli) = crate::managers::llm::cli::Cli::from_base_url(&provider.base_url) {
        return Ok(cli.models().iter().map(|m| m.to_string()).collect());
    }
    let base_url = provider.base_url.trim_end_matches('/');
    let url = format!("{}/models", base_url);

    debug!("Fetching models from: {}", url);

    let client = create_client(provider, &api_key)?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch models: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!(
            "Model list request failed ({}): {}",
            status, error_text
        ));
    }

    let parsed: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let mut models = Vec::new();

    // Handle OpenAI format: { data: [ { id: "..." }, ... ] }
    if let Some(data) = parsed.get("data").and_then(|d| d.as_array()) {
        for entry in data {
            if let Some(id) = entry.get("id").and_then(|i| i.as_str()) {
                models.push(id.to_string());
            } else if let Some(name) = entry.get("name").and_then(|n| n.as_str()) {
                models.push(name.to_string());
            }
        }
    }
    // Handle array format: [ "model1", "model2", ... ]
    else if let Some(array) = parsed.as_array() {
        for entry in array {
            if let Some(model) = entry.as_str() {
                models.push(model.to_string());
            }
        }
    }

    Ok(models)
}

// ---------------------------------------------------- Ollama, nativ, auf CPU --

/// Ollamas eigener Endpunkt aus einer OpenAI-kompatiblen Basis-URL.
///
/// Der kompatible Pfad ist `…/v1/chat/completions`, der native
/// `…/api/chat`. Nur der native nimmt `options` entgegen — und nur dort
/// lässt sich die GPU abwählen.
///
/// `None`, wenn die Basis nicht nach einem lokalen Dienst aussieht: bei
/// OpenAI oder Groq stellt sich die Frage nicht, und vLLM kennt `/api/chat`
/// gar nicht.
pub fn ollama_native_url(base_url: &str) -> Option<String> {
    let base = base_url.trim_end_matches('/');
    if !(base.contains("localhost") || base.contains("127.0.0.1")) {
        return None;
    }
    let root = base.strip_suffix("/v1").unwrap_or(base);
    Some(format!("{root}/api/chat"))
}

/// Eine Anfrage über Ollamas nativen Endpunkt — wahlweise ohne GPU.
///
/// `num_gpu: 0` verlegt das Modell vollständig in den Arbeitsspeicher. Das
/// ist langsamer, aber der Fish-Speech-Server belegt rund 17 GB
/// Grafikspeicher; ein zweites Modell daneben bringt beide zum Straucheln
/// oder den Treiber zum Absturz.
///
/// `keep_alive: 0` entlädt das Modell direkt nach der Antwort. Der warme
/// Zwischenspeicher wäre bequem — aber neben dem Fish-Speech-Server ist
/// jedes Gigabyte eines, das fehlt, und die Übersetzungen selbst liegen
/// ohnehin je Text und Sprache auf Platte: ein Sprachwechsel zurück kostet
/// keinen zweiten Modelllauf.
pub async fn send_ollama_native(
    purpose: Purpose,
    provider: &PostProcessProvider,
    url: &str,
    model: &str,
    prompt: String,
    cpu_only: bool,
) -> Result<Option<String>, String> {
    // Regelwerk vor dem Budget: ein gesperrter Anbieter bekommt gar nichts.
    crate::managers::compliance::check_call(provider)?;
    usage::check_budget(provider, model)?;
    let started = std::time::Instant::now();
    let (result, tokens) = match ollama_native_inner(url, model, prompt, cpu_only).await {
        Ok((content, tokens)) => (Ok(content), tokens),
        Err(e) => (Err(e), TokenUsage::default()),
    };
    let elapsed = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
    usage::record_call(
        purpose,
        provider,
        model,
        tokens,
        elapsed,
        result.as_ref().map(|_| ()).map_err(|e| e.clone()),
    )
    .await;
    result
}

async fn ollama_native_inner(
    url: &str,
    model: &str,
    prompt: String,
    cpu_only: bool,
) -> Result<(Option<String>, TokenUsage), String> {
    let mut options = serde_json::json!({});
    if cpu_only {
        options["num_gpu"] = serde_json::json!(0);
    }
    let body = serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": prompt }],
        "stream": false,
        "keep_alive": 0,
        "options": options,
    });
    let client = reqwest::Client::new();
    let response = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Ollama request failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Ollama answered {}", response.status()));
    }
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Ollama response not JSON: {e}"))?;
    // Ollamas eigene Zaehler: `prompt_eval_count` und `eval_count`.
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

/// Ein bei Ollama geladenes Modell sofort entladen (best effort).
///
/// Der OpenAI-kompatible Pfad kennt kein `keep_alive`: eine Übersetzung, die
/// darüber lief, lässt das Modell nach Ollamas Voreinstellung noch Minuten
/// im Speicher stehen — Grafikspeicher, den der Fish-Speech-Server braucht.
/// Ein leerer `/api/generate`-Aufruf mit `keep_alive: 0` räumt es ab.
///
/// Scheitern ist kein Fehler des Aufrufers: dann läuft eben Ollamas eigene
/// Frist ab. Deshalb kein Rückgabewert, nur ein Protokolleintrag.
pub async fn ollama_unload(base_url: &str, model: &str) {
    let Some(chat_url) = ollama_native_url(base_url) else {
        return;
    };
    let url = chat_url.replace("/api/chat", "/api/generate");
    let body = serde_json::json!({ "model": model, "keep_alive": 0 });
    match reqwest::Client::new().post(&url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            log::info!("Ollama-Modell '{model}' nach der Übersetzung entladen");
        }
        Ok(resp) => log::warn!("Ollama unload answered {}", resp.status()),
        Err(e) => log::warn!("Ollama unload failed: {e}"),
    }
}

// ------------------------------------------------ Streaming (M4, P4c Chat) --

/// Eine Nachricht fuer `send_chat_completion_stream` (`role` = `system` |
/// `user` | `assistant`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StreamMessage {
    pub role: String,
    pub content: String,
}

impl StreamMessage {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.to_string(),
            content: content.into(),
        }
    }
}

/// Mehr Antworttext nimmt der Stream nicht an: ein Modell in einer
/// Wiederholungsschleife soll weder den Speicher noch die Oberflaeche fluten.
/// Beim Erreichen wird die Verbindung geschlossen (der Server bricht dann ab).
pub const STREAM_MAX_CHARS: usize = 32 * 1024;
/// Laengste Pause zwischen zwei Stream-Stuecken, sobald Text fliesst. Vor dem
/// ersten Token gilt `STREAM_FIRST_TOKEN_TIMEOUT` (Prefill auf CPU dauert).
const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
const STREAM_FIRST_TOKEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
/// Eine SSE-Zeile laenger als das ist kein Token-Stream mehr.
const SSE_MAX_LINE_BYTES: usize = 1024 * 1024;

/// Fester Startwert fuer den lokalen Chat (mit `temperature` 0 ohnehin nur
/// Absicherung, falls ein Server trotzdem sampelt).
pub const CHAT_SEED: u32 = 42;

/// Anfrage-Body fuer Chat mit oder ohne Streaming. Beim lokalen Server wird
/// das Denken des Modells abgeschaltet (Qwen3/Qwen3.5: `enable_thinking=false`
/// ueber `chat_template_kwargs`) und die Antwort deterministisch gemacht
/// (`temperature` 0, fester `seed`): dieselbe Frage mit denselben Auszuegen
/// gibt dieselbe Antwort, und ein kleines Modell zitiert nicht mal so, mal so
/// (P4g: Eval-Genauigkeit 0,58-0,71 je Lauf). Entfernte Anbieter bekommen
/// diese Felder nicht, weil manche unbekannte Felder oder Werte mit 400
/// ablehnen (Denkmodelle erlauben keine Temperatur).
pub fn stream_request_body(
    model: &str,
    messages: &[StreamMessage],
    stream: bool,
    local: bool,
) -> Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": stream,
    });
    if local {
        body["chat_template_kwargs"] = serde_json::json!({ "enable_thinking": false });
        body["temperature"] = serde_json::json!(0);
        body["seed"] = serde_json::json!(CHAT_SEED);
    }
    body
}

/// Ein Ereignis aus dem SSE-Strom eines OpenAI-kompatiblen Servers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseEvent {
    Delta(String),
    Usage(TokenUsage),
    Done,
    /// Der Server meldet einen Fehler im Strom (`{"error": ...}`); nur der
    /// Typ/Code, nie der Text der Anfrage.
    Error(String),
}

/// Zerlegt SSE-Bytes in Ereignisse. Puffert bis zum Zeilenende, damit ein
/// UTF-8-Zeichen, das auf zwei Netzwerkpakete verteilt ist, heil bleibt (ein
/// `\n` kommt in UTF-8 nie mitten in einem Zeichen vor).
#[derive(Default)]
pub struct SseDecoder {
    buf: Vec<u8>,
}

impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, String> {
        self.buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            if let Some(event) = parse_sse_line(line.trim_end_matches(['\r', '\n'])) {
                events.push(event);
            }
        }
        if self.buf.len() > SSE_MAX_LINE_BYTES {
            return Err("SSE-Zeile zu lang".to_string());
        }
        Ok(events)
    }

    /// Rest ohne abschliessenden Zeilenumbruch (Server schliesst direkt nach
    /// dem letzten `data:`).
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let rest = std::mem::take(&mut self.buf);
        let line = String::from_utf8_lossy(&rest);
        parse_sse_line(line.trim_end_matches(['\r', '\n']))
            .into_iter()
            .collect()
    }
}

fn parse_sse_line(line: &str) -> Option<SseEvent> {
    let payload = line.strip_prefix("data:")?.trim();
    if payload.is_empty() {
        return None;
    }
    if payload == "[DONE]" {
        return Some(SseEvent::Done);
    }
    let value: Value = serde_json::from_str(payload).ok()?;
    if let Some(error) = value.get("error") {
        let code = error
            .get("type")
            .or_else(|| error.get("code"))
            .map(|c| c.to_string())
            .unwrap_or_else(|| "unbekannt".to_string());
        return Some(SseEvent::Error(code));
    }
    if let Some(content) = value
        .pointer("/choices/0/delta/content")
        .and_then(|c| c.as_str())
    {
        if !content.is_empty() {
            return Some(SseEvent::Delta(content.to_string()));
        }
    }
    if let Some(usage) = value.get("usage").filter(|u| u.is_object()) {
        let count = |key: &str| usage.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
        return Some(SseEvent::Usage(TokenUsage {
            prompt_tokens: count("prompt_tokens"),
            completion_tokens: count("completion_tokens"),
        }));
    }
    None
}

/// Chat-Anfrage mit Streaming (SSE, `stream: true`). `on_delta` bekommt jedes
/// Textstueck in Reihenfolge; das Ergebnis ist der ganze Text.
///
/// Rueckfall: lehnt der Anbieter Streaming ab (Fehlerstatus) oder bricht der
/// Strom VOR dem ersten Token ab, wird die Anfrage einmal ohne Streaming
/// gestellt und die ganze Antwort als ein Stueck gemeldet. Schickt der Server
/// trotz `stream: true` eine ganze JSON-Antwort, wird sie genauso genommen.
/// Nach dem ersten Token ist ein Abbruch ein Fehler (der Text war schon zu
/// sehen). Den Abbruch durch den Nutzer macht der Aufrufer, indem er das
/// Future fallen laesst: die Verbindung schliesst, der Server hoert auf.
///
/// Gebucht wird wie bei `send_chat_completion_with_schema`: jeder Aufruf,
/// auch ein gescheiterter (dann mit null Token). Kein Prompt- oder
/// Antworttext im Log.
pub async fn send_chat_completion_stream(
    purpose: Purpose,
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    messages: Vec<StreamMessage>,
    on_delta: &(dyn Fn(&str) + Send + Sync),
) -> Result<String, String> {
    // Regelwerk vor dem Budget: ein gesperrter Anbieter bekommt gar nichts.
    crate::managers::compliance::check_call(provider)?;
    usage::check_budget(provider, model)?;
    let started = std::time::Instant::now();
    let (result, tokens) = match stream_inner(provider, api_key, model, messages, on_delta).await {
        Ok((content, tokens)) => (Ok(content), tokens),
        Err(e) => (Err(e), TokenUsage::default()),
    };
    let elapsed = started.elapsed().as_millis().min(u32::MAX as u128) as u32;
    usage::record_call(
        purpose,
        provider,
        model,
        tokens,
        elapsed,
        result.as_ref().map(|_| ()).map_err(|e| e.clone()),
    )
    .await;
    result
}

async fn stream_inner(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    messages: Vec<StreamMessage>,
    on_delta: &(dyn Fn(&str) + Send + Sync),
) -> Result<(String, TokenUsage), String> {
    if let Some(cli) = crate::managers::llm::cli::Cli::from_base_url(&provider.base_url) {
        // Die CLI liefert am Stueck: Systemteile als Systemtext, der Rest als
        // Gespraech mit Rollen.
        let system: Vec<&str> = messages
            .iter()
            .filter(|m| m.role == "system")
            .map(|m| m.content.as_str())
            .collect();
        let dialog: Vec<String> = messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect();
        let system = (!system.is_empty()).then(|| system.join("\n\n"));
        let (reply, tokens) = send_cli(cli, model, dialog.join("\n\n"), system, None).await?;
        let text = reply.content.unwrap_or_default();
        on_delta(&text);
        return Ok((text, tokens));
    }
    use futures_util::StreamExt;

    let resolved = crate::managers::llm::resolve_base_url(provider, model).await?;
    let url = format!("{}/chat/completions", resolved.trim_end_matches('/'));
    debug!("Sending streaming chat completion request to: {}", url);
    let client = create_client(provider, &api_key)?;
    let local = crate::managers::llm::is_local(provider);
    let fallback_body = stream_request_body(model, &messages, false, local);

    let response = client
        .post(&url)
        .json(&stream_request_body(model, &messages, true, local))
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        log::info!("Streaming abgelehnt ({status}), Anfrage ohne Streaming");
        return complete_once(&client, &url, &fallback_body, on_delta).await;
    }
    let is_sse = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));
    if !is_sse {
        // Der Server hat `stream` ignoriert und eine ganze Antwort geschickt.
        let completion: ChatCompletionResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse API response: {}", e))?;
        return Ok(deliver_whole(completion, on_delta));
    }

    let mut decoder = SseDecoder::default();
    let mut text = String::new();
    let mut text_chars = 0usize;
    let mut tokens = TokenUsage::default();
    let mut stream = response.bytes_stream();
    'read: loop {
        let wait = if text.is_empty() {
            STREAM_FIRST_TOKEN_TIMEOUT
        } else {
            STREAM_IDLE_TIMEOUT
        };
        let (events, ended) = match tokio::time::timeout(wait, stream.next()).await {
            Err(_) => return Err("Stream: Zeitlimit ohne neue Daten".to_string()),
            Ok(None) => (decoder.finish(), true),
            Ok(Some(Err(e))) => {
                if text.is_empty() {
                    log::info!("Stream vor dem ersten Token abgebrochen, Anfrage ohne Streaming");
                    return complete_once(&client, &url, &fallback_body, on_delta).await;
                }
                return Err(format!("Stream abgebrochen: {e}"));
            }
            Ok(Some(Ok(bytes))) => (decoder.push(&bytes)?, false),
        };
        for event in events {
            match event {
                SseEvent::Delta(delta) => {
                    let delta_chars = delta.chars().count();
                    if text_chars + delta_chars > STREAM_MAX_CHARS {
                        log::warn!(
                            "Stream: Antwort ueber {STREAM_MAX_CHARS} Zeichen, abgeschnitten"
                        );
                        break 'read;
                    }
                    text_chars += delta_chars;
                    text.push_str(&delta);
                    on_delta(&delta);
                }
                SseEvent::Usage(u) => tokens = u,
                SseEvent::Done => break 'read,
                SseEvent::Error(code) => {
                    return Err(format!("Stream-Fehler vom Server ({code})"));
                }
            }
        }
        if ended {
            break;
        }
    }
    Ok((text, tokens))
}

fn deliver_whole(
    completion: ChatCompletionResponse,
    on_delta: &(dyn Fn(&str) + Send + Sync),
) -> (String, TokenUsage) {
    let text: String = completion
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default()
        .chars()
        .take(STREAM_MAX_CHARS)
        .collect();
    if !text.is_empty() {
        on_delta(&text);
    }
    (text, TokenUsage::from(completion.usage))
}

/// Rueckfall ohne Streaming: dieselben Nachrichten, `stream: false`.
async fn complete_once(
    client: &reqwest::Client,
    url: &str,
    body: &Value,
    on_delta: &(dyn Fn(&str) + Send + Sync),
) -> Result<(String, TokenUsage), String> {
    let response = client
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;
    let status = response.status();
    if !status.is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read error response".to_string());
        return Err(format!(
            "API request failed with status {}: {}",
            status, error_text
        ));
    }
    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse API response: {}", e))?;
    Ok(deliver_whole(completion, on_delta))
}

// ---------------------------------------------------------------------------
// D3: Bildanfrage (Folien) an einen OpenAI-kompatiblen Server mit Projektor
// ---------------------------------------------------------------------------
//
// Regeln aus dem Spike M7 (`koordination/bild-video/spike/spike-bericht.md`):
// - KEIN `response_format: json_schema` bei Bildanfragen: Gemma 4 E4B lief damit
//   auf echten Bildern bis `max_tokens` in eine Leerzeichen-Schleife (18 von 35
//   Antworten unbrauchbar). Der Aufrufer fragt nach einfachem Zeilenformat und
//   wertet selbst aus.
// - `max_tokens` klein halten (Beschreibung <= 200); ein Bild kostet ~1 000
//   Eingabe-Token, die der Server dank `-b/-ub 2048` in einem Batch sieht.
// - Denken aus (`enable_thinking: false`), wie bei den Textaufrufen.
// Kein Verbrauchs-Ledger: der Aufruf geht nur an den lokalen Server und kostet kein Geld.

/// Hoechste Bilddatei, die gesendet wird (ein 1080p-JPEG hat ~100 kB).
pub const IMAGE_MAX_BYTES: usize = 8 * 1024 * 1024;
/// Obergrenze fuer `max_tokens` einer Bildanfrage.
pub const IMAGE_MAX_TOKENS: u32 = 1024;
/// Wartezeit einer Bildanfrage: 2 s auf der GPU, ~100 s auf der CPU.
const IMAGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// MIME-Typ einer Bilddatei nach Endung; `None` fuer alles, was kein Bild ist.
pub fn image_mime(path: &std::path::Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Der Body einer Bildanfrage: eine Nutzernachricht aus Bild (`image_url` mit
/// base64-Data-URL) und Text. Rein, damit der Aufbau testbar ist.
pub fn image_request_body(
    model: &str,
    prompt: &str,
    mime: &str,
    image: &[u8],
    max_tokens: u32,
) -> Value {
    use base64::Engine as _;
    let data_url = format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(image)
    );
    serde_json::json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "image_url", "image_url": { "url": data_url } },
                { "type": "text", "text": prompt },
            ],
        }],
        "temperature": 0.1,
        "max_tokens": max_tokens.clamp(1, IMAGE_MAX_TOKENS),
        "chat_template_kwargs": { "enable_thinking": false },
    })
}

/// Antwort einer Bildanfrage: der Text (leer, wenn der Server keinen lieferte) und ob
/// er abgeschnitten wurde (`finish_reason == "length"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReply {
    pub content: String,
    pub truncated: bool,
}

/// Schickt `image` samt `prompt` an `<base_url>/chat/completions` (`base_url` endet auf
/// `/v1`). Fehlertexte tragen einen Code als Praefix: `image_unreadable`,
/// `image_unsupported`, `image_empty`, `image_too_large`, `request_failed`, `bad_response`.
pub async fn post_image_prompt(
    base_url: &str,
    model: &str,
    image: &std::path::Path,
    prompt: &str,
    max_tokens: u32,
) -> Result<ImageReply, String> {
    let mime =
        image_mime(image).ok_or_else(|| "image_unsupported: kein JPEG/PNG/WebP".to_string())?;
    let bytes = std::fs::read(image).map_err(|e| format!("image_unreadable: {e}"))?;
    if bytes.is_empty() {
        return Err("image_empty: Bilddatei ist leer".to_string());
    }
    if bytes.len() > IMAGE_MAX_BYTES {
        return Err(format!("image_too_large: {} Byte", bytes.len()));
    }
    let body = image_request_body(model, prompt, mime, &bytes, max_tokens);
    drop(bytes);
    let client = reqwest::Client::builder()
        .timeout(IMAGE_TIMEOUT)
        .build()
        .map_err(|e| format!("request_failed: {e}"))?;
    let response = client
        .post(format!("{}/chat/completions", base_url.trim_end_matches('/')))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request_failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        let head: String = text.chars().take(300).collect();
        return Err(format!("request_failed: HTTP {status}: {head}"));
    }
    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|e| format!("bad_response: {e}"))?;
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

#[cfg(test)]
mod stream_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn provider(port: u16) -> PostProcessProvider {
        PostProcessProvider {
            id: "custom".into(),
            label: "Test".into(),
            base_url: format!("http://127.0.0.1:{port}/v1"),
            allow_base_url_edit: true,
            models_endpoint: None,
            supports_structured_output: false,
        }
    }

    /// Antwort des Roh-Mocks: Status, Content-Type und Stuecke, die mit einer
    /// kleinen Pause einzeln geschrieben werden (Body endet mit dem Schliessen).
    pub(super) struct RawReply {
        pub(super) status: u16,
        pub(super) content_type: &'static str,
        pub(super) chunks: Vec<Vec<u8>>,
    }

    /// Mock-Server: `handler(n, body)` bekommt die laufende Nummer der Anfrage
    /// und ihren Body.
    pub(super) async fn spawn_raw_mock(
        handler: impl Fn(usize, &str) -> RawReply + Send + Sync + 'static,
    ) -> u16 {
        let handler = Arc::new(handler);
        let counter = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let handler = Arc::clone(&handler);
                let n = counter.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 1 << 20];
                    let mut read = 0usize;
                    let body = loop {
                        let got = sock.read(&mut buf[read..]).await.unwrap_or(0);
                        if got == 0 {
                            return;
                        }
                        read += got;
                        let text = String::from_utf8_lossy(&buf[..read]).to_string();
                        if let Some(end) = text.find("\r\n\r\n") {
                            let len = text[..end]
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase()
                                        .strip_prefix("content-length: ")
                                        .map(|v| v.trim().to_string())
                                })
                                .and_then(|v| v.parse::<usize>().ok())
                                .unwrap_or(0);
                            if read >= end + 4 + len {
                                break String::from_utf8_lossy(&buf[end + 4..end + 4 + len])
                                    .to_string();
                            }
                        }
                    };
                    let reply = handler(n, &body);
                    let head = format!(
                        "HTTP/1.1 {} Mock\r\nContent-Type: {}\r\nConnection: close\r\n\r\n",
                        reply.status, reply.content_type
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    for chunk in reply.chunks {
                        let _ = sock.write_all(&chunk).await;
                        let _ = sock.flush().await;
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                    let _ = sock.shutdown().await;
                });
            }
        });
        port
    }

    fn sse(parts: &[&str]) -> Vec<Vec<u8>> {
        parts
            .iter()
            .map(|p| {
                format!(
                    "data: {}\n\n",
                    serde_json::json!({"choices": [{"delta": {"content": p}}]})
                )
                .into_bytes()
            })
            .collect()
    }

    fn collect() -> (Arc<Mutex<Vec<String>>>, impl Fn(&str) + Send + Sync) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        (seen, move |d: &str| {
            sink.lock().unwrap().push(d.to_string())
        })
    }

    #[test]
    fn sse_decoder_keeps_split_lines_and_split_utf8_intact() {
        let line = format!(
            "data: {}\n\n",
            serde_json::json!({"choices": [{"delta": {"content": "Grüße"}}]})
        );
        let bytes = line.as_bytes();
        // Mitten im "ü" (2 Byte) teilen.
        let cut = line.find('ü').unwrap() + 1;
        let mut dec = SseDecoder::default();
        assert!(dec.push(&bytes[..cut]).unwrap().is_empty());
        let events = dec.push(&bytes[cut..]).unwrap();
        assert_eq!(events, vec![SseEvent::Delta("Grüße".into())]);
    }

    #[test]
    fn sse_decoder_reads_usage_done_errors_and_ignores_noise() {
        let mut dec = SseDecoder::default();
        let input = concat!(
            ": keep-alive\r\n",
            "event: message\r\n",
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\r\n\r\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\r\n\r\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":3}}\n\n",
            "data: [DONE]\n\n",
            "data: {\"error\":{\"type\":\"server_error\",\"message\":\"geheim\"}}\n",
        );
        let events = dec.push(input.as_bytes()).unwrap();
        assert_eq!(
            events,
            vec![
                SseEvent::Delta("A".into()),
                SseEvent::Usage(TokenUsage {
                    prompt_tokens: 12,
                    completion_tokens: 3
                }),
                SseEvent::Done,
                SseEvent::Error("\"server_error\"".into()),
            ]
        );
        // Letzte Zeile ohne Umbruch kommt mit `finish`.
        assert!(dec.push(b"data: [DONE]").unwrap().is_empty());
        assert_eq!(dec.finish(), vec![SseEvent::Done]);
        // Riesige Zeile ohne Umbruch: Fehler statt unbegrenztem Puffer.
        let mut dec = SseDecoder::default();
        assert!(dec.push(&vec![b'x'; SSE_MAX_LINE_BYTES + 1]).is_err());
    }

    #[test]
    fn the_local_body_turns_thinking_off_and_the_remote_body_does_not() {
        let msgs = vec![StreamMessage::new("user", "Frage")];
        let local = stream_request_body("m", &msgs, true, true);
        assert_eq!(local["stream"], true);
        assert_eq!(local["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(local["messages"][0]["role"], "user");
        // Lokal deterministisch: Temperatur 0 und fester Startwert.
        assert_eq!(local["temperature"], 0);
        assert_eq!(local["seed"], CHAT_SEED);
        let remote = stream_request_body("m", &msgs, false, false);
        assert_eq!(remote["stream"], false);
        assert!(remote.get("chat_template_kwargs").is_none());
        // Entfernte Anbieter bekommen keine Sampling-Felder.
        assert!(remote.get("temperature").is_none());
        assert!(remote.get("seed").is_none());
    }

    // ---- P1g: Denkmodus aus fuer Qwen3/Qwen3.5 ---------------------------

    fn request_json(local: bool, model: &str) -> Value {
        request_json_for(local, model, Purpose::PostProcess)
    }

    fn request_json_for(local: bool, model: &str, purpose: Purpose) -> Value {
        let sampling = deterministic_sampling(local, purpose);
        let request = ChatCompletionRequest {
            model: model.to_string(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: "Frage".into(),
            }],
            response_format: None,
            reasoning_effort: None,
            reasoning: None,
            chat_template_kwargs: thinking_off_kwargs(local, model),
            temperature: sampling.0,
            seed: sampling.1,
        };
        serde_json::to_value(&request).unwrap()
    }

    #[test]
    fn qwen3_and_qwen35_get_thinking_switched_off_locally() {
        for model in ["llm-qwen3.5-9b-q4", "llm-qwen3-4b-q4", "Qwen3-8B-Q4_K_M"] {
            let body = request_json(true, model);
            assert_eq!(
                body["chat_template_kwargs"]["enable_thinking"], false,
                "{model}"
            );
        }
    }

    #[test]
    fn other_local_models_and_remote_providers_keep_the_request_unchanged() {
        for model in ["llm-gemma4-e4b-q4", "llm-gemma4-12b-q4", "llama-3.1-8b"] {
            let body = request_json(true, model);
            assert!(body.get("chat_template_kwargs").is_none(), "{model}");
        }
        // Auch ein Qwen3 bei einem entfernten Anbieter: kein unbekanntes Feld.
        let remote = request_json(false, "qwen3-235b-a22b");
        assert!(remote.get("chat_template_kwargs").is_none());
        // Sonst nur die bisherigen Felder.
        let body = request_json(true, "llm-gemma4-e4b-q4");
        let mut keys: Vec<&str> = body.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["messages", "model"]);
    }

    #[test]
    fn local_meeting_notes_calls_are_deterministic_and_nothing_else_is() {
        // Lokal + KI-Notizen: Temperatur 0 und fester Startwert (wie P4g im Chat).
        let body = request_json_for(true, "llm-gemma4-e4b-q4", Purpose::EnhancedNotes);
        assert_eq!(body["temperature"].as_f64(), Some(0.0));
        assert_eq!(body["seed"], CHAT_SEED);
        // P1k: das Protokoll (und mit ihm die Vorlagenwahl) ebenso.
        let body = request_json_for(true, "llm-gemma4-e4b-q4", Purpose::Minutes);
        assert_eq!(body["temperature"].as_f64(), Some(0.0));
        assert_eq!(body["seed"], CHAT_SEED);
        let remote = request_json_for(false, "gpt-4.1", Purpose::Minutes);
        assert!(remote.get("temperature").is_none() && remote.get("seed").is_none());
        // Auch bei einem Denkmodell zusammen mit abgeschaltetem Denken.
        let body = request_json_for(true, "llm-qwen3.5-9b-q4", Purpose::EnhancedNotes);
        assert_eq!(body["temperature"].as_f64(), Some(0.0));
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        // Entfernte Anbieter: keine Sampling-Felder (Denkmodelle lehnen sie ab).
        let remote = request_json_for(false, "gpt-4.1", Purpose::EnhancedNotes);
        assert!(remote.get("temperature").is_none() && remote.get("seed").is_none());
        // Follow-up-Mail (B13): ebenfalls deterministisch, nur lokal.
        let mail = request_json_for(true, "llm-gemma4-e4b-q4", Purpose::Followup);
        assert_eq!(mail["temperature"].as_f64(), Some(0.0));
        assert_eq!(mail["seed"], CHAT_SEED);
        let mail_remote = request_json_for(false, "gpt-4.1", Purpose::Followup);
        assert!(mail_remote.get("temperature").is_none() && mail_remote.get("seed").is_none());
        // G5: die Uebersetzung einer Transkript-Fassung ist ebenfalls deterministisch (nur lokal).
        let translation =
            request_json_for(true, "llm-gemma4-e4b-q4", Purpose::TranscriptTranslation);
        assert_eq!(translation["temperature"].as_f64(), Some(0.0));
        assert_eq!(translation["seed"], CHAT_SEED);
        let translation_remote =
            request_json_for(false, "gpt-4.1", Purpose::TranscriptTranslation);
        assert!(
            translation_remote.get("temperature").is_none()
                && translation_remote.get("seed").is_none()
        );
        // Andere lokale Zwecke (Diktat, Tagging, ...) bleiben unveraendert.
        for purpose in [Purpose::PostProcess, Purpose::Tagging, Purpose::Translation, Purpose::Summary] {
            let other = request_json_for(true, "llm-gemma4-e4b-q4", purpose);
            assert!(other.get("temperature").is_none() && other.get("seed").is_none(), "{purpose:?}");
        }
    }

    #[test]
    fn think_leftovers_are_removed_from_local_answers_only() {
        let raw = Some("<think>\nerst nachdenken\n</think>\n\n{\"a\":1}".to_string());
        assert_eq!(clean_local_content(true, raw.clone()).as_deref(), Some("{\"a\":1}"));
        // Nur ein schliessendes Tag (Vorlage hat das oeffnende schon gesetzt).
        let tail = Some("Gedanken</think>\n{\"a\":1}".to_string());
        assert_eq!(clean_local_content(true, tail).as_deref(), Some("{\"a\":1}"));
        // Nicht geschlossen: nichts Brauchbares dahinter.
        let open = Some("<think>ohne Ende".to_string());
        assert_eq!(clean_local_content(true, open).as_deref(), Some(""));
        // Ohne Tags, ohne Antwort, entfernt: unveraendert.
        assert_eq!(
            clean_local_content(true, Some("{\"a\":1}".into())).as_deref(),
            Some("{\"a\":1}")
        );
        assert_eq!(clean_local_content(true, None), None);
        assert_eq!(clean_local_content(false, raw.clone()), raw);
    }

    #[tokio::test]
    async fn stream_delivers_deltas_in_order_and_returns_the_whole_text() {
        let port = spawn_raw_mock(|_, body| {
            assert!(body.contains("\"stream\":true"), "Streaming angefragt");
            let mut chunks = sse(&["Das ", "Budget ", "steht [Q1]."]);
            chunks.push(b"data: [DONE]\n\n".to_vec());
            RawReply {
                status: 200,
                content_type: "text/event-stream",
                chunks,
            }
        })
        .await;
        let (seen, on_delta) = collect();
        let text = send_chat_completion_stream(
            Purpose::Chat,
            &provider(port),
            String::new(),
            "m",
            vec![StreamMessage::new("user", "x")],
            &on_delta,
        )
        .await
        .unwrap();
        assert_eq!(text, "Das Budget steht [Q1].");
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["Das ", "Budget ", "steht [Q1]."]
        );
    }

    #[tokio::test]
    async fn a_stream_without_done_ends_with_the_connection() {
        let port = spawn_raw_mock(|_, _| RawReply {
            status: 200,
            content_type: "text/event-stream",
            chunks: sse(&["Ende ", "ohne DONE"]),
        })
        .await;
        let (_seen, on_delta) = collect();
        let text = send_chat_completion_stream(
            Purpose::Chat,
            &provider(port),
            String::new(),
            "m",
            vec![StreamMessage::new("user", "x")],
            &on_delta,
        )
        .await
        .unwrap();
        assert_eq!(text, "Ende ohne DONE");
    }

    #[tokio::test]
    async fn a_plain_json_answer_counts_as_one_delta() {
        let port = spawn_raw_mock(|_, _| RawReply {
            status: 200,
            content_type: "application/json",
            chunks: vec![serde_json::json!({
                "choices": [{"message": {"role": "assistant", "content": "Alles auf einmal"}}]
            })
            .to_string()
            .into_bytes()],
        })
        .await;
        let (seen, on_delta) = collect();
        let text = send_chat_completion_stream(
            Purpose::Chat,
            &provider(port),
            String::new(),
            "m",
            vec![StreamMessage::new("user", "x")],
            &on_delta,
        )
        .await
        .unwrap();
        assert_eq!(text, "Alles auf einmal");
        assert_eq!(*seen.lock().unwrap(), vec!["Alles auf einmal"]);
    }

    #[tokio::test]
    async fn a_refused_stream_falls_back_to_one_non_streaming_request() {
        let port = spawn_raw_mock(|n, body| {
            if n == 0 {
                assert!(body.contains("\"stream\":true"));
                RawReply {
                    status: 400,
                    content_type: "text/plain",
                    chunks: vec![b"stream not supported".to_vec()],
                }
            } else {
                assert!(
                    body.contains("\"stream\":false"),
                    "Rueckfall ohne Streaming"
                );
                RawReply {
                    status: 200,
                    content_type: "application/json",
                    chunks: vec![serde_json::json!({
                        "choices": [{"message": {"content": "Rueckfall"}}],
                        "usage": {"prompt_tokens": 5, "completion_tokens": 1}
                    })
                    .to_string()
                    .into_bytes()],
                }
            }
        })
        .await;
        let (seen, on_delta) = collect();
        let text = send_chat_completion_stream(
            Purpose::Chat,
            &provider(port),
            String::new(),
            "m",
            vec![StreamMessage::new("user", "x")],
            &on_delta,
        )
        .await
        .unwrap();
        assert_eq!(text, "Rueckfall");
        assert_eq!(*seen.lock().unwrap(), vec!["Rueckfall"]);
    }

    #[tokio::test]
    async fn an_error_after_the_first_token_is_a_failure_not_a_second_request() {
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let port = spawn_raw_mock(move |_, _| {
            counter.fetch_add(1, Ordering::SeqCst);
            let mut chunks = sse(&["Teil "]);
            chunks.push(b"data: {\"error\":{\"type\":\"server_error\"}}\n\n".to_vec());
            RawReply {
                status: 200,
                content_type: "text/event-stream",
                chunks,
            }
        })
        .await;
        let (seen, on_delta) = collect();
        let err = send_chat_completion_stream(
            Purpose::Chat,
            &provider(port),
            String::new(),
            "m",
            vec![StreamMessage::new("user", "x")],
            &on_delta,
        )
        .await
        .unwrap_err();
        assert!(err.contains("server_error"), "war: {err}");
        assert_eq!(*seen.lock().unwrap(), vec!["Teil "]);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_runaway_answer_is_cut_at_the_limit() {
        let port = spawn_raw_mock(|_, _| {
            let piece = "x".repeat(4096);
            let parts: Vec<&str> = (0..12).map(|_| piece.as_str()).collect();
            RawReply {
                status: 200,
                content_type: "text/event-stream",
                chunks: sse(&parts),
            }
        })
        .await;
        let (_seen, on_delta) = collect();
        let text = send_chat_completion_stream(
            Purpose::Chat,
            &provider(port),
            String::new(),
            "m",
            vec![StreamMessage::new("user", "x")],
            &on_delta,
        )
        .await
        .unwrap();
        assert!(text.chars().count() <= STREAM_MAX_CHARS);
        assert!(text.chars().count() >= STREAM_MAX_CHARS - 4096);
    }
}

#[cfg(test)]
mod image_tests {
    use super::stream_tests::{spawn_raw_mock, RawReply};
    use super::*;
    use std::sync::{Arc, Mutex};

    fn json_reply(content: Value, finish: &str) -> RawReply {
        let body = serde_json::json!({
            "choices": [{ "message": { "content": content }, "finish_reason": finish }]
        });
        RawReply {
            status: 200,
            content_type: "application/json",
            chunks: vec![body.to_string().into_bytes()],
        }
    }

    fn temp_image(name: &str, bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    /// Der Body: Nachricht aus Bild (base64-Data-URL) und Text, KEIN `response_format`
    /// (Spike: Schema-Zwang laeuft bei Bildern in Leerzeichen), kleines `max_tokens`,
    /// Denken aus.
    #[test]
    fn the_request_carries_a_data_url_and_never_a_json_schema() {
        use base64::Engine as _;
        let png: &[u8] = &[0x89, b'P', b'N', b'G', 1, 2];
        let body = image_request_body("llm-gemma4-e4b-q4", "Was steht da?", "image/png", png, 200);
        assert_eq!(body["model"], "llm-gemma4-e4b-q4");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
        let content = &body["messages"][0]["content"];
        assert_eq!(content[0]["type"], "image_url");
        let expected = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        );
        assert_eq!(content[0]["image_url"]["url"].as_str(), Some(expected.as_str()));
        assert_eq!(
            content[1],
            serde_json::json!({ "type": "text", "text": "Was steht da?" })
        );
        assert!(
            body.get("response_format").is_none(),
            "kein json_schema bei Bildern"
        );
        assert!(body.get("stream").is_none());
        assert_eq!(body["max_tokens"], 200);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        // Die Obergrenze gilt auch, wenn der Aufrufer mehr (oder nichts) verlangt.
        let big = image_request_body("m", "p", "image/jpeg", b"x", 100_000);
        assert_eq!(big["max_tokens"], IMAGE_MAX_TOKENS);
        let zero = image_request_body("m", "p", "image/jpeg", b"x", 0);
        assert_eq!(zero["max_tokens"], 1);
    }

    #[test]
    fn mime_follows_the_extension_and_rejects_non_images() {
        let mime = |n: &str| image_mime(std::path::Path::new(n));
        assert_eq!(mime("a.jpg"), Some("image/jpeg"));
        assert_eq!(mime("a.JPEG"), Some("image/jpeg"));
        assert_eq!(mime("a.png"), Some("image/png"));
        assert_eq!(mime("a.webp"), Some("image/webp"));
        assert_eq!(mime("a.gif"), None);
        assert_eq!(mime("a.txt"), None);
        assert_eq!(mime("a"), None);
    }

    /// Ende zu Ende gegen einen Test-Server: der Server sieht genau den Body, die Antwort kommt an.
    #[tokio::test]
    async fn a_reply_comes_back_and_the_server_saw_the_image() {
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = Arc::clone(&seen);
        let port = spawn_raw_mock(move |_, body| {
            sink.lock().unwrap().push(body.to_string());
            json_reply(
                Value::String("sprecher_raum\nEine Person am Pult.".into()),
                "stop",
            )
        })
        .await;
        let (_dir, image) = temp_image("0001.jpg", b"jpegbytes");
        let reply = post_image_prompt(
            &format!("http://127.0.0.1:{port}/v1"),
            "m",
            &image,
            "Frage",
            200,
        )
        .await
        .unwrap();
        assert_eq!(reply.content, "sprecher_raum\nEine Person am Pult.");
        assert!(!reply.truncated);
        let sent: Value = serde_json::from_str(&seen.lock().unwrap()[0]).unwrap();
        assert_eq!(sent["messages"][0]["content"][1]["text"], "Frage");
        assert!(sent["messages"][0]["content"][0]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/jpeg;base64,"));
    }

    /// Abgeschnitten, null und Leerzeichen: kein Fehler der Verbindung, der Aufrufer entscheidet.
    #[tokio::test]
    async fn truncated_null_and_blank_replies_are_reported_not_hidden() {
        let port = spawn_raw_mock(|n, _| match n {
            0 => json_reply(Value::String("abc".into()), "length"),
            1 => json_reply(Value::Null, "stop"),
            _ => json_reply(Value::String("   ".into()), "stop"),
        })
        .await;
        let (_dir, image) = temp_image("a.png", b"png");
        let url = format!("http://127.0.0.1:{port}/v1");
        // Die Mock-Nummer zaehlt die Verbindungen: nacheinander abfragen.
        let first = post_image_prompt(&url, "m", &image, "p", 50).await.unwrap();
        assert_eq!((first.content.as_str(), first.truncated), ("abc", true));
        let second = post_image_prompt(&url, "m", &image, "p", 50).await.unwrap();
        assert_eq!((second.content.as_str(), second.truncated), ("", false));
        let third = post_image_prompt(&url, "m", &image, "p", 50).await.unwrap();
        assert_eq!(
            third.content, "   ",
            "Leerzeichen-Antwort wird unveraendert geliefert"
        );
    }

    #[tokio::test]
    async fn bad_images_and_server_errors_carry_a_code() {
        let port = spawn_raw_mock(|_, _| RawReply {
            status: 500,
            content_type: "text/plain",
            chunks: vec![b"kaputt".to_vec()],
        })
        .await;
        let url = format!("http://127.0.0.1:{port}/v1");
        let (_dir, image) = temp_image("a.jpg", b"x");
        let err = post_image_prompt(&url, "m", &image, "p", 50)
            .await
            .unwrap_err();
        assert!(err.starts_with("request_failed: HTTP 500"), "{err}");
        assert!(err.contains("kaputt"), "{err}");

        let (_d2, empty) = temp_image("e.jpg", b"");
        let err = post_image_prompt(&url, "m", &empty, "p", 50)
            .await
            .unwrap_err();
        assert!(err.starts_with("image_empty"), "{err}");
        let (_d3, text) = temp_image("a.txt", b"x");
        let err = post_image_prompt(&url, "m", &text, "p", 50)
            .await
            .unwrap_err();
        assert!(err.starts_with("image_unsupported"), "{err}");
        let missing = std::path::Path::new("Z:/gibt/es/nicht.jpg");
        let err = post_image_prompt(&url, "m", missing, "p", 50)
            .await
            .unwrap_err();
        assert!(err.starts_with("image_unreadable"), "{err}");
        // Kein Server: Verbindungsfehler, kein Haenger.
        let err = post_image_prompt("http://127.0.0.1:1/v1", "m", &image, "p", 50)
            .await
            .unwrap_err();
        assert!(err.starts_with("request_failed"), "{err}");
    }
}
