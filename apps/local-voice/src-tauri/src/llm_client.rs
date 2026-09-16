/// Missing configuration is an actionable setup state, not an inference failure.
pub const MODEL_SETUP_REQUIRED: &str = "language_model_setup_required: Choose or install a language model under Models, or connect a provider in Settings.";

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
    // Hartes Budget: die Verweigerung ist bewusst ungebucht -- es wurde ja
    // nichts verbraucht.
    usage::check_budget(provider, model)?;
    let started = std::time::Instant::now();
    let (result, tokens) = match send_inner(
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
    );
    result
}

/// Der eigentliche Aufruf; liefert Antwort und Token-Zaehler getrennt, damit
/// der Aufrufer oben beides buchen kann.
#[allow(clippy::too_many_arguments)]
async fn send_inner(
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<Value>,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
) -> Result<(Option<String>, TokenUsage), String> {
    if provider.id == crate::settings::APPLE_INTELLIGENCE_PROVIDER_ID
        || provider.base_url.starts_with("apple-intelligence://")
    {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let mut instructions = system_prompt.unwrap_or_default();
            if let Some(schema) = json_schema {
                instructions.push_str("\nReturn only valid JSON matching this schema, without Markdown fences:\n");
                instructions.push_str(&schema.to_string());
            }
            let output = tauri::async_runtime::spawn_blocking(move || {
                crate::apple_intelligence::generate_text(&instructions, &user_content)
            }).await.map_err(|e| e.to_string())??;
            return Ok((Some(output), TokenUsage::default()));
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        return Err("Apple Intelligence requires an Apple silicon Mac, macOS 26 or later and an available system model.".into());
    }
    // Fuer den lokalen Anbieter ist die Adresse erst bekannt, wenn der
    // Server laeuft -- und der wird hier bei Bedarf gestartet.
    let resolved = crate::managers::llm::resolve_base_url(provider, model).await?;
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
    Ok((
        completion
            .choices
            .first()
            .and_then(|choice| choice.message.content.clone()),
        tokens,
    ))
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
    );
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
