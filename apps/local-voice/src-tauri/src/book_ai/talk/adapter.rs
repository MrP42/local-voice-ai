//! Modell-Adapter für das Eval-Tor (lva#324): derselbe Weg wie in der App,
//! aber ohne `AppHandle`, Verbrauchsbuch und Regelwerk.
//!
//! - `ollama`, `openai`: OpenAI-kompatibler Server (`/chat/completions`),
//!   Anfragekörper wie `llm_client::stream_request_body`, Ausgabegrenze wie
//!   in der App (`llm_client::apply_limits`). Ohne exakten Zähler: das
//!   Ergebnis ist `found_only` mit `budget`, es geht kein Aufruf raus.
//! - `local_server`: laufender llama-server (`--url http://127.0.0.1:PORT`),
//!   mit exaktem Zähler (`count::LlamaCounter`) wie der lokale Anbieter der App.
//! - `claude_cli`, `codex_cli`: die isolierte CLI-Anbindung
//!   (`managers::llm::cli`), Nutzernachricht mit Rollenvorsatz wie im
//!   `llm_client`.
//! - `fake`: kein Modell, die Antworten stehen in der Fixture.
//!
//! Kein Frage-, Antwort- oder Buchtext im Log.

use std::time::Duration;

use serde_json::Value;

use super::count::LlamaCounter;
use super::prompt::CountFuture;
use super::{map_error_code, ModelFuture, TalkError, TalkModel, CODE_LLM_FAILED, CODE_OFFLINE};
use super::{CODE_TIMEOUT, DEFAULT_MAX_ANSWER_TOKENS};
use crate::llm_client::{apply_limits, stream_request_body, StreamLimits, StreamMessage};
use crate::managers::llm::cli::{self, CallOptions, Cli};

/// Kennungen der Anbieter, die `book_talk_eval` kennt.
pub const PROVIDERS: &[&str] = &[
    "fake",
    "ollama",
    "openai",
    "local_server",
    "claude_cli",
    "codex_cli",
];
/// Zeitgrenze eines CLI-Aufrufs.
const CLI_TIMEOUT: Duration = Duration::from_secs(180);
/// Zeitgrenze einer HTTP-Anfrage.
const HTTP_TIMEOUT: Duration = Duration::from_secs(180);

/// OpenAI-kompatibler Server.
pub struct HttpModel {
    client: reqwest::Client,
    url: String,
    model: String,
    api_key: Option<String>,
    local: bool,
    ollama: bool,
    context_tokens: u32,
    counter: Option<LlamaCounter>,
}

impl HttpModel {
    pub fn new(
        base_url: &str,
        model: &str,
        api_key: Option<String>,
        local: bool,
        ollama: bool,
        context_tokens: u32,
    ) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| format!("HTTP-Client: {e}"))?;
        Ok(Self {
            client,
            url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
            model: model.to_string(),
            api_key: api_key.filter(|k| !k.is_empty()),
            local,
            ollama,
            context_tokens,
            counter: None,
        })
    }

    /// Mit exaktem Zähler des llama-servers hinter `base_url`.
    pub fn with_counter(mut self, base_url: &str) -> Self {
        self.counter = LlamaCounter::new(base_url);
        self
    }
}

fn reply_text(body: &Value) -> Option<String> {
    body.get("choices")?
        .get(0)?
        .get("message")?
        .get("content")?
        .as_str()
        .map(str::to_string)
}

impl TalkModel for HttpModel {
    fn complete<'a>(
        &'a self,
        system: &'a str,
        user: String,
        on_delta: &'a (dyn Fn(&str) + Send + Sync),
    ) -> ModelFuture<'a> {
        Box::pin(async move {
            let messages = vec![
                StreamMessage::new("system", system),
                StreamMessage::new("user", user),
            ];
            let mut body = stream_request_body(&self.model, &messages, false, self.local);
            apply_limits(
                &mut body,
                self.ollama,
                &StreamLimits {
                    max_tokens: Some(DEFAULT_MAX_ANSWER_TOKENS),
                    num_ctx: Some(self.context_tokens),
                    strict: true,
                },
            );
            let mut request = self.client.post(&self.url).json(&body);
            if let Some(key) = &self.api_key {
                request = request.bearer_auth(key);
            }
            let response = request.send().await.map_err(|e| {
                TalkError::new(if e.is_timeout() {
                    CODE_TIMEOUT
                } else if e.is_connect() {
                    CODE_OFFLINE
                } else {
                    CODE_LLM_FAILED
                })
            })?;
            let status = response.status();
            let text = response
                .text()
                .await
                .map_err(|_| TalkError::new(CODE_LLM_FAILED))?;
            if !status.is_success() {
                return Err(TalkError::new(map_error_code(&text)));
            }
            let reply = serde_json::from_str::<Value>(&text)
                .ok()
                .as_ref()
                .and_then(reply_text)
                .ok_or_else(|| TalkError::new(CODE_LLM_FAILED))?;
            on_delta(&reply);
            Ok(reply)
        })
    }

    fn token_count<'a>(&'a self, system: &'a str, user: &'a str) -> CountFuture<'a> {
        Box::pin(async move { self.counter.as_ref()?.count(system, user).await })
    }

    fn context_tokens(&self) -> CountFuture<'_> {
        Box::pin(async move { self.counter.as_ref()?.context().await })
    }
}

/// Abo-Modell über die CLI (Claude Code oder Codex).
pub struct CliModel {
    cli: Cli,
    binary: std::path::PathBuf,
    model: String,
}

impl CliModel {
    pub fn new(kind: &str, model: &str) -> Result<Self, String> {
        let cli = Cli::from_kind(kind).ok_or_else(|| format!("unbekannte CLI: {kind}"))?;
        let binary = cli::locate(cli)
            .ok_or_else(|| format!("{kind}: die CLI ist nicht installiert (nicht im PATH)"))?;
        Ok(Self {
            cli,
            binary,
            model: model.to_string(),
        })
    }
}

impl TalkModel for CliModel {
    fn complete<'a>(
        &'a self,
        system: &'a str,
        user: String,
        on_delta: &'a (dyn Fn(&str) + Send + Sync),
    ) -> ModelFuture<'a> {
        Box::pin(async move {
            let (cli, binary, model) = (self.cli, self.binary.clone(), self.model.clone());
            let system = system.to_string();
            let dialog = format!("user: {user}");
            let reply = tokio::task::spawn_blocking(move || {
                cli::call_with_options(
                    cli,
                    &binary,
                    &model,
                    Some(&system),
                    &dialog,
                    CLI_TIMEOUT,
                    &CallOptions {
                        cancel: None,
                        max_tokens: Some(DEFAULT_MAX_ANSWER_TOKENS),
                        require_usage: true,
                        preview: None,
                        preview_switch: None,
                        stdout_zeile: None,
                    },
                )
            })
            .await
            .map_err(|_| TalkError::new(CODE_LLM_FAILED))?
            .map_err(|e| TalkError::new(map_error_code(&e)))?;
            on_delta(&reply.text);
            Ok(reply.text)
        })
    }
}

/// Baut das Modell zu einer Anbieterkennung. `Ok(None)` für `fake`.
pub fn build_model(
    provider: &str,
    base_url: Option<&str>,
    model: &str,
    api_key: Option<String>,
    context_tokens: u32,
) -> Result<Option<Box<dyn TalkModel>>, String> {
    match provider {
        "fake" => Ok(None),
        "local_server" => {
            let url = base_url
                .ok_or_else(|| "local_server: --url http://127.0.0.1:PORT fehlt".to_string())?;
            let base = format!("{}/v1", url.trim_end_matches('/').trim_end_matches("/v1"));
            let model = HttpModel::new(&base, model, None, true, false, context_tokens)?
                .with_counter(&base);
            Ok(Some(Box::new(model)))
        }
        "ollama" | "openai" => {
            let default = if provider == "ollama" {
                "http://127.0.0.1:11434/v1"
            } else {
                "https://api.openai.com/v1"
            };
            let model = HttpModel::new(
                base_url.unwrap_or(default),
                model,
                api_key,
                provider == "ollama",
                provider == "ollama",
                context_tokens,
            )?;
            Ok(Some(Box::new(model)))
        }
        "claude_cli" | "codex_cli" => Ok(Some(Box::new(CliModel::new(provider, model)?))),
        other => Err(format!(
            "unbekannter Anbieter: {other} (erlaubt: {})",
            PROVIDERS.join(", ")
        )),
    }
}
